//! Built-in OPC UA simulator (`kind = "opcua"`).
//!
//! An embedded `async-opcua` server (anonymous, security `None`) whose variables are backed
//! by the simulator's own state through read/write callbacks — so the harness has exact
//! ground truth (current values, write counters, seeded bad-status nodes) without poking at
//! the server's address space. A [`TransportProxy`] fronts the server, and the server
//! *advertises the proxy's port* in its endpoints: OPC UA clients (re)connect to the
//! advertised endpoint URL, so without this the session would bypass the proxy and
//! transport-drop checks (B5) would test nothing.
//!
//! A seed with a `security` section makes the server secured (doc/connectors/
//! opcua-connector-spec.md): it presents a certificate generated for the run, offers the
//! listed policy/mode pairs, accepts the listed username and an X.509 user, and hands the
//! connector a PKI directory (`[connection] pki_dir`) that already trusts the server. Values
//! `"@password"`, `"@user_certificate"` and `"@user_private_key"` in a device's
//! `protocol_address` are replaced with the generated credentials.

use super::proxy::TransportProxy;
use super::{PointData, PointSpec, Simulator};
use opcua::nodes::VariableBuilder;
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use connector_opcua::pki::{CertificateRequest, Group, Pki};
use opcua::crypto::SecurityPolicy;
use opcua::crypto::Thumbprint;
use opcua::server::authenticator::{AuthManager, DefaultAuthenticator, Password, UserToken};
use opcua::server::{
    ServerBuilder, ServerEndpoint, ServerHandle, ServerUserToken, ANONYMOUS_USER_TOKEN_ID,
};
use opcua::types::MessageSecurityMode;
use opcua::types::{DataTypeId, DataValue, DateTime, NodeId, ObjectId, StatusCode, UAString, Variant};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tedge_dot_sdk::DataType;
use tracing::debug;

const NAMESPACE_URI: &str = "urn:tedge-dot-conformance:opcua";
/// The index the custom namespace lands on: the application URI is always namespace 1, and
/// the simulator registers exactly one namespace after it. Connector configs address nodes
/// as `{ namespace = 2, identifier = "..." }`.
const NAMESPACE_INDEX: u16 = 2;

/// Seed file: the variables the server exposes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    /// Free-text description; ignored.
    #[serde(default, rename = "$comment")]
    _comment: Option<String>,
    variables: Vec<SeedVariable>,
    #[serde(default)]
    security: Option<SeedSecurity>,
}

/// A secured simulator (see the module documentation).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedSecurity {
    /// `[policy, mode]` pairs the server offers besides None, e.g.
    /// `["Basic256Sha256", "sign_and_encrypt"]`.
    endpoints: Vec<(String, String)>,
    /// The username the server accepts (`"@password"` is its password).
    user: String,
}

const SERVER_URI: &str = "urn:tedge-dot-conformance";

/// The credentials and directories of a secured simulator.
struct Secured {
    /// Holds every generated file; removed with the simulator.
    dir: crate::host::TempDir,
    password: String,
}

impl Secured {
    fn client_pki(&self) -> std::path::PathBuf {
        self.dir.path().join("client-pki")
    }
    fn user_certificate(&self) -> std::path::PathBuf {
        self.dir.path().join("user/own/certs/cert.der")
    }
    fn user_private_key(&self) -> std::path::PathBuf {
        self.dir.path().join("user/own/private/key.pem")
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedVariable {
    /// String node identifier within the simulator namespace.
    identifier: String,
    datatype: DataType,
    value: serde_json::Value,
    #[serde(default)]
    writable: bool,
    /// Reads of this node answer with `BadSensorFailure` (behavioural check B4).
    #[serde(default)]
    bad_status: bool,
}

struct VarState {
    variant: Variant,
    bad_status: bool,
    writes: usize,
}

type SharedState = Arc<Mutex<HashMap<String, VarState>>>;

pub struct OpcuaSim {
    secured: Option<Secured>,
    state: SharedState,
    outage: Arc<AtomicBool>,
    proxy: TransportProxy,
    handle: ServerHandle,
}

impl Drop for OpcuaSim {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

impl OpcuaSim {
    pub async fn start(seed_path: &Path) -> Result<OpcuaSim, String> {
        let text = std::fs::read_to_string(seed_path)
            .map_err(|e| format!("failed to read seed '{}': {e}", seed_path.display()))?;
        let seed: Seed = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse seed '{}': {e}", seed_path.display()))?;

        let mut state = HashMap::new();
        for var in &seed.variables {
            state.insert(
                var.identifier.clone(),
                VarState {
                    variant: variant_from_seed(var.datatype, &var.value)?,
                    bad_status: var.bad_status,
                    writes: 0,
                },
            );
        }
        let state: SharedState = Arc::new(Mutex::new(state));
        let outage = Arc::new(AtomicBool::new(false));

        // The server listens on an internal port; the proxy in front owns the port the
        // connector sees AND the port the server advertises.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("opcua sim bind failed: {e}"))?;
        let internal_port = listener
            .local_addr()
            .map_err(|e| format!("opcua sim local_addr: {e}"))?
            .port();
        let proxy = TransportProxy::start(internal_port).await?;

        let mut builder = ServerBuilder::new_anonymous("tedge-dot-conformance");
        let mut secured = None;
        if let Some(security) = &seed.security {
            let (b, s) = secure_server(builder, security)?;
            builder = b;
            secured = Some(s);
        }
        let (server, handle) = builder
            .application_uri(SERVER_URI)
            .product_uri(SERVER_URI)
            .host("127.0.0.1")
            .port(proxy.port())
            .discovery_urls(vec![format!("opc.tcp://127.0.0.1:{}", proxy.port())])
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: NAMESPACE_URI.to_owned(),
                    ..Default::default()
                },
                "simple",
            ))
            .build()
            .map_err(|e| format!("opcua sim server build failed: {e}"))?;

        let nm = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or("opcua sim: no SimpleNodeManager")?;
        let ns = handle
            .get_namespace_index(NAMESPACE_URI)
            .ok_or("opcua sim: namespace not registered")?;
        if ns != NAMESPACE_INDEX {
            return Err(format!(
                "opcua sim: namespace landed on index {ns}, expected {NAMESPACE_INDEX} \
                 (connector configs hardcode it)"
            ));
        }

        for var in &seed.variables {
            let node = NodeId::new(ns, var.identifier.as_str());
            {
                let mut space = nm.address_space().write();
                let mut builder = VariableBuilder::new(&node, &var.identifier, &var.identifier)
                    .data_type(data_type_id(var.datatype))
                    .value(variant_from_seed(var.datatype, &var.value)?)
                    .organized_by(ObjectId::ObjectsFolder);
                if var.writable {
                    builder = builder.writable();
                }
                builder.insert(&mut *space);
            }

            // Serve reads from the simulator state (with outage / bad-status overrides)...
            let read_state = state.clone();
            let read_outage = outage.clone();
            let read_id = var.identifier.clone();
            nm.inner().add_read_callback(node.clone(), move |_, _, _| {
                if read_outage.load(Ordering::SeqCst) {
                    return Err(StatusCode::BadInternalError);
                }
                let state = read_state.lock().unwrap();
                let var = state.get(&read_id).ok_or(StatusCode::BadNodeIdUnknown)?;
                if var.bad_status {
                    return Err(StatusCode::BadSensorFailure);
                }
                Ok(DataValue {
                    value: Some(var.variant.clone()),
                    status: Some(StatusCode::Good),
                    source_timestamp: Some(DateTime::now()),
                    server_timestamp: Some(DateTime::now()),
                    ..Default::default()
                })
            });

            // ...and record writes into it (the callback replaces the default node store, so
            // the simulator is the single source of truth). Access levels are enforced by
            // the server before the callback runs.
            if var.writable {
                let write_state = state.clone();
                let write_id = var.identifier.clone();
                nm.inner().add_write_callback(node.clone(), move |dv, _| {
                    let Some(variant) = dv.value else {
                        return StatusCode::BadNothingToDo;
                    };
                    let mut state = write_state.lock().unwrap();
                    let Some(var) = state.get_mut(&write_id) else {
                        return StatusCode::BadNodeIdUnknown;
                    };
                    var.variant = variant;
                    var.writes += 1;
                    StatusCode::Good
                });
            }
        }

        tokio::spawn(async move {
            if let Err(e) = server.run_with(listener).await {
                debug!("opcua sim server stopped: {e}");
            }
        });

        Ok(OpcuaSim {
            secured,
            state,
            outage,
            proxy,
            handle,
        })
    }

    fn lookup<T>(
        &self,
        point: &PointSpec,
        f: impl FnOnce(&VarState) -> Result<T, String>,
    ) -> Result<T, String> {
        let identifier = identifier_from(&point.address)?;
        let state = self.state.lock().unwrap();
        let var = state
            .get(&identifier)
            .ok_or_else(|| format!("no seeded variable '{identifier}'"))?;
        f(var)
    }
}

#[async_trait::async_trait]
impl Simulator for OpcuaSim {
    fn port(&self) -> u16 {
        self.proxy.port()
    }

    fn point_data(&self, point: &PointSpec) -> Result<PointData, String> {
        self.lookup(point, |var| {
            if var.bad_status {
                return Err("seeded bad-status variable".into());
            }
            Ok(PointData {
                bytes: variant_raw_bytes(&var.variant)?,
                raw_group: 1,
            })
        })
    }

    fn is_invalid(&self, point: &PointSpec) -> bool {
        self.lookup(point, |var| Ok(var.bad_status)).unwrap_or(false)
    }

    fn write_count(&self, point: &PointSpec) -> Result<usize, String> {
        self.lookup(point, |var| Ok(var.writes))
    }

    fn set_outage(&self, on: bool) {
        self.outage.store(on, Ordering::SeqCst);
    }

    async fn set_transport(&self, up: bool) -> Result<(), String> {
        self.proxy.set_up(up).await
    }

    async fn set_stalled(&self, stalled: bool) -> Result<(), String> {
        self.proxy.set_stalled(stalled);
        Ok(())
    }

    fn rewrite_protocol_address(&self, address: &mut toml::Value) -> Result<(), String> {
        let table = address
            .as_table_mut()
            .ok_or("device protocol_address is not a table")?;
        table.insert(
            "endpoint".into(),
            toml::Value::String(format!("opc.tcp://127.0.0.1:{}", self.proxy.port())),
        );
        for (_, value) in table.iter_mut() {
            let Some(text) = value.as_str() else { continue };
            if !text.starts_with('@') {
                continue;
            }
            let secured = self
                .secured
                .as_ref()
                .ok_or_else(|| format!("'{text}' needs a seed with a security section"))?;
            *value = toml::Value::String(match text {
                "@password" => secured.password.clone(),
                "@user_certificate" => secured.user_certificate().display().to_string(),
                "@user_private_key" => secured.user_private_key().display().to_string(),
                other => return Err(format!("unknown placeholder '{other}'")),
            });
        }
        Ok(())
    }

    fn connection_overrides(&self) -> Vec<(String, toml::Value)> {
        match &self.secured {
            Some(secured) => vec![(
                "pki_dir".into(),
                toml::Value::String(secured.client_pki().display().to_string()),
            )],
            None => Vec::new(),
        }
    }
}

/// async-opcua's default authenticator advertises its X.509 user token policy with the
/// deprecated Basic128Rsa15, which open62541 (the C build) no longer offers, so the token
/// could never be signed. This one advertises (and verifies with) the endpoint's own policy,
/// as servers commonly do.
struct EndpointPolicyAuthenticator(DefaultAuthenticator);

#[async_trait::async_trait]
impl AuthManager for EndpointPolicyAuthenticator {
    async fn authenticate_anonymous_token(&self, endpoint: &ServerEndpoint) -> Result<(), opcua::types::Error> {
        self.0.authenticate_anonymous_token(endpoint).await
    }

    async fn authenticate_username_identity_token(
        &self,
        endpoint: &ServerEndpoint,
        username: &str,
        password: &Password,
    ) -> Result<UserToken, opcua::types::Error> {
        self.0.authenticate_username_identity_token(endpoint, username, password).await
    }

    async fn authenticate_x509_identity_token(
        &self,
        endpoint: &ServerEndpoint,
        signing_thumbprint: &Thumbprint,
    ) -> Result<UserToken, opcua::types::Error> {
        self.0.authenticate_x509_identity_token(endpoint, signing_thumbprint).await
    }

    fn user_token_policies(&self, endpoint: &ServerEndpoint) -> Vec<opcua::types::UserTokenPolicy> {
        let mut policies = self.0.user_token_policies(endpoint);
        for policy in &mut policies {
            if policy.token_type == opcua::types::UserTokenType::Certificate {
                policy.security_policy_uri = endpoint.security_policy().to_uri().into();
            }
        }
        policies
    }
}

/// Make the server secured: a generated certificate, the seeded endpoints and users, and a
/// connector PKI directory that trusts the server certificate.
fn secure_server(
    builder: ServerBuilder,
    security: &SeedSecurity,
) -> Result<(ServerBuilder, Secured), String> {
    let dir = crate::host::TempDir::new()?;
    let request = |name: &str, uri: &str| CertificateRequest {
        application_name: name.into(),
        application_uri: uri.into(),
        hostnames: vec!["127.0.0.1".into(), "localhost".into()],
        days: 30,
    };
    let server = Pki::new(dir.path().join("server"))
        .create(&request("tedge-dot conformance server", SERVER_URI), false)?;
    let user = Pki::new(dir.path().join("user"))
        .create(&request("conformance operator", "urn:tedge-dot-conformance:user"), false)?;
    let client = Pki::new(dir.path().join("client-pki"));
    client.ensure_layout()?;
    let der = server
        .certificate
        .to_der()
        .map_err(|_| "cannot encode the server certificate".to_string())?;
    client.store(Group::Trusted, &der)?;

    let password = format!("conformance-{}", std::process::id());
    let user_tokens = std::collections::BTreeMap::from([
        ("user".to_string(), ServerUserToken::user_pass(security.user.clone(), password.clone())),
        (
            "x509".to_string(),
            ServerUserToken {
                user: "x509".into(),
                x509: Some(user.certificate_path.display().to_string()),
                // The server fills this in for ITS copy of the tokens only; the
                // authenticator below gets its own.
                thumbprint: Some(user.certificate.thumbprint()),
                ..Default::default()
            },
        ),
    ]);
    let users = vec![
        ANONYMOUS_USER_TOKEN_ID.to_string(),
        "user".to_string(),
        "x509".to_string(),
    ];
    let mut builder = builder
        .create_sample_keypair(false)
        .pki_dir(dir.path().join("server-store"))
        .certificate_path(&server.certificate_path)
        .private_key_path(&server.private_key_path)
        .trust_client_certs(true)
        .with_authenticator(std::sync::Arc::new(EndpointPolicyAuthenticator(
            DefaultAuthenticator::new(user_tokens.clone()),
        )));
    for (id, token) in user_tokens {
        builder = builder.add_user_token(id, token);
    }
    for (i, (policy, mode)) in security.endpoints.iter().enumerate() {
        let policy = connector_opcua::Policy::parse(policy)
            .map(|p| SecurityPolicy::from_uri(&p.uri()))
            .ok_or_else(|| format!("seed: unknown policy '{policy}'"))?;
        let mode = match mode.as_str() {
            "sign" => MessageSecurityMode::Sign,
            "sign_and_encrypt" => MessageSecurityMode::SignAndEncrypt,
            other => return Err(format!("seed: unknown mode '{other}'")),
        };
        builder = builder.add_endpoint(
            format!("secured-{i}"),
            ServerEndpoint::new("/", policy, mode, &users),
        );
    }
    Ok((builder, Secured { dir, password }))
}

/// The point's node identifier within the simulator namespace, from its `address` object.
fn identifier_from(address: &serde_json::Value) -> Result<String, String> {
    if let Some(ns) = address.get("namespace").and_then(|n| n.as_u64()) {
        if ns != NAMESPACE_INDEX as u64 {
            return Err(format!(
                "point addresses namespace {ns}; the simulator serves namespace {NAMESPACE_INDEX}"
            ));
        }
    }
    address
        .get("identifier")
        .and_then(|i| i.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("opcua point address needs a string 'identifier': {address}"))
}

fn data_type_id(dt: DataType) -> DataTypeId {
    match dt {
        DataType::Bool => DataTypeId::Boolean,
        DataType::Int8 => DataTypeId::SByte,
        DataType::Uint8 => DataTypeId::Byte,
        DataType::Int16 => DataTypeId::Int16,
        DataType::Uint16 => DataTypeId::UInt16,
        DataType::Int32 => DataTypeId::Int32,
        DataType::Uint32 => DataTypeId::UInt32,
        DataType::Int64 => DataTypeId::Int64,
        DataType::Uint64 => DataTypeId::UInt64,
        DataType::Float32 => DataTypeId::Float,
        DataType::Float64 => DataTypeId::Double,
        DataType::String | DataType::Bytes => DataTypeId::String,
    }
}

/// Build the seeded `Variant` for a variable (mirrors the connector's write coercion).
fn variant_from_seed(dt: DataType, value: &serde_json::Value) -> Result<Variant, String> {
    let err = || format!("seed value {value} is not valid for datatype {dt:?}");
    Ok(match dt {
        DataType::Bool => Variant::Boolean(value.as_bool().ok_or_else(err)?),
        DataType::Int8 => Variant::SByte(value.as_i64().ok_or_else(err)? as i8),
        DataType::Uint8 => Variant::Byte(value.as_u64().ok_or_else(err)? as u8),
        DataType::Int16 => Variant::Int16(value.as_i64().ok_or_else(err)? as i16),
        DataType::Uint16 => Variant::UInt16(value.as_u64().ok_or_else(err)? as u16),
        DataType::Int32 => Variant::Int32(value.as_i64().ok_or_else(err)? as i32),
        DataType::Uint32 => Variant::UInt32(value.as_u64().ok_or_else(err)? as u32),
        DataType::Int64 => Variant::Int64(value.as_i64().ok_or_else(err)?),
        DataType::Uint64 => Variant::UInt64(value.as_u64().ok_or_else(err)?),
        DataType::Float32 => Variant::Float(value.as_f64().ok_or_else(err)? as f32),
        DataType::Float64 => Variant::Double(value.as_f64().ok_or_else(err)?),
        DataType::String => {
            Variant::String(UAString::from(value.as_str().ok_or_else(err)?.to_string()))
        }
        DataType::Bytes => return Err("datatype 'bytes' is not seedable over OPC UA".into()),
    })
}

/// The raw byte echo the connector reports for a variant (big-endian value bytes — must stay
/// in lockstep with `connector-opcua`'s `variant_to_value`).
fn variant_raw_bytes(v: &Variant) -> Result<Vec<u8>, String> {
    Ok(match v {
        Variant::Boolean(b) => vec![*b as u8],
        Variant::SByte(i) => vec![*i as u8],
        Variant::Byte(u) => vec![*u],
        Variant::Int16(i) => i.to_be_bytes().to_vec(),
        Variant::UInt16(u) => u.to_be_bytes().to_vec(),
        Variant::Int32(i) => i.to_be_bytes().to_vec(),
        Variant::UInt32(u) => u.to_be_bytes().to_vec(),
        Variant::Int64(i) => i.to_be_bytes().to_vec(),
        Variant::UInt64(u) => u.to_be_bytes().to_vec(),
        Variant::Float(f) => f.to_be_bytes().to_vec(),
        Variant::Double(d) => d.to_be_bytes().to_vec(),
        Variant::String(s) => s.as_ref().to_string().into_bytes(),
        other => return Err(format!("unsupported variant {other:?}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tedge_dot_sdk::Mode;

    fn write_seed(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("ot-conf-opcua-{}-{name}", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn spec(identifier: &str, datatype: DataType) -> PointSpec {
        PointSpec {
            address: serde_json::json!({ "namespace": 2, "identifier": identifier }),
            datatype: Some(datatype),
            mode: Mode::Typed,
        }
    }

    #[tokio::test]
    async fn serves_seeded_variables_and_ground_truth() {
        let seed = write_seed(
            "basic.json",
            r#"{
                "variables": [
                    { "identifier": "Temperature", "datatype": "float64", "value": 21.5 },
                    { "identifier": "Broken", "datatype": "uint16", "value": 0, "bad_status": true }
                ]
            }"#,
        );
        let sim = OpcuaSim::start(&seed).await.unwrap();

        let temp = sim.point_data(&spec("Temperature", DataType::Float64)).unwrap();
        assert_eq!(temp.bytes, 21.5f64.to_be_bytes().to_vec());
        assert_eq!(temp.raw_group, 1);
        assert!(!sim.is_invalid(&spec("Temperature", DataType::Float64)));
        assert!(sim.is_invalid(&spec("Broken", DataType::Uint16)));
        assert_eq!(sim.write_count(&spec("Temperature", DataType::Float64)).unwrap(), 0);

        std::fs::remove_file(&seed).ok();
    }
}
