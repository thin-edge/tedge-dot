//! Secured sessions end to end (doc/connectors/opcua-connector-spec.md §3–§8): the real
//! `OpcuaConnector` against an in-process `async-opcua` server with secured endpoints, username
//! and X.509 users, and server certificates from the shared PKI vectors (genpki.py).

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use connector_opcua::pki::{Group, Pki};
use connector_opcua::{security, OpcuaConnector};
use opcua::crypto::SecurityPolicy;
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::nodes::VariableBuilder;
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::{
    ServerBuilder, ServerEndpoint, ServerHandle, ServerUserToken, ANONYMOUS_USER_TOKEN_ID,
};
use opcua::types::{DataTypeId, MessageSecurityMode, NodeId, ObjectId};
use tedge_dot_sdk::{
    library, Access, Connector, DataType, Endianness, LinkReport, LinkStatus, Mode, PointRef,
    Quality, WordOrder,
};
use tokio::net::TcpListener;

const SERVER_URI: &str = "urn:tedge:opcua-sim";
const PASSWORD: &str = "operator-secret-pw";

struct TestServer {
    handle: ServerHandle,
    port: u16,
    /// The server's own PKI directory (where it keeps client certificates it rejected).
    pki: PathBuf,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

struct ServerOptions<'a> {
    /// The genpki scenario whose server certificate the server presents.
    scenario: &'a str,
    /// The host the server advertises in its endpoint URLs.
    advertised_host: &'a str,
    /// Accept any client application certificate.
    trust_clients: bool,
    /// Longest secure channel token the server grants, in milliseconds. `None` leaves the
    /// library default (an hour), which no test can outlive.
    max_token_lifetime_ms: Option<u32>,
}

impl Default for ServerOptions<'_> {
    fn default() -> Self {
        ServerOptions {
            scenario: "pinned",
            advertised_host: "127.0.0.1",
            trust_clients: true,
            max_token_lifetime_ms: None,
        }
    }
}

/// Endpoints: None (anonymous, and a username whose password travels unencrypted),
/// Basic256Sha256 sign / sign-and-encrypt and Aes256_Sha256_RsaPss sign-and-encrypt (anonymous,
/// username, X.509 user).
async fn start_server(vectors: &Path, opts: ServerOptions<'_>) -> TestServer {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let scenario = vectors.join("scenarios").join(opts.scenario);
    let pki = common::tempdir("server-pki");

    let all_users: Vec<String> = vec![
        ANONYMOUS_USER_TOKEN_ID.to_string(),
        "operator".to_string(),
        "x509-operator".to_string(),
    ];
    let mut none = ServerEndpoint::new_none("/", &all_users);
    none.password_security_policy = Some(SecurityPolicy::None.to_string());
    let secured = |policy, mode| ServerEndpoint::new("/", policy, mode, &all_users);

    let builder = ServerBuilder::new()
        .application_name("tedge-dot secured test server")
        .application_uri(SERVER_URI)
        .product_uri(SERVER_URI)
        .create_sample_keypair(false)
        .pki_dir(&pki)
        .certificate_path(scenario.join("server/cert.der"))
        .private_key_path(scenario.join("server/key.pem"))
        .trust_client_certs(opts.trust_clients)
        .host(opts.advertised_host)
        .port(port)
        .discovery_urls(vec![format!("opc.tcp://{}:{port}/", opts.advertised_host)])
        .add_user_token("operator", ServerUserToken::user_pass("operator", PASSWORD))
        .add_user_token(
            "x509-operator",
            ServerUserToken {
                user: "x509-operator".into(),
                x509: Some(vectors.join("users/operator.der").display().to_string()),
                ..Default::default()
            },
        )
        .add_endpoint("none", none)
        .add_endpoint(
            "b256-sign",
            secured(SecurityPolicy::Basic256Sha256, MessageSecurityMode::Sign),
        )
        .add_endpoint(
            "b256-sign-encrypt",
            secured(
                SecurityPolicy::Basic256Sha256,
                MessageSecurityMode::SignAndEncrypt,
            ),
        )
        .add_endpoint(
            "aes256-sign-encrypt",
            secured(
                SecurityPolicy::Aes256Sha256RsaPss,
                MessageSecurityMode::SignAndEncrypt,
            ),
        )
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:tedge-dot-opcua-test:nodes".to_owned(),
                ..Default::default()
            },
            "simple",
        ));
    let builder = match opts.max_token_lifetime_ms {
        Some(ms) => builder.max_secure_channel_token_lifetime_ms(ms),
        None => builder,
    };
    let (server, handle) = builder.build().unwrap();

    // One readable node, so a test can tell "the channel still works" from "the read failed".
    {
        let nm = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();
        let ns = handle
            .get_namespace_index("urn:tedge-dot-opcua-test:nodes")
            .unwrap();
        let mut space = nm.address_space().write();
        VariableBuilder::new(&NodeId::new(ns, "T"), "T", "T")
            .data_type(DataTypeId::Double)
            .value(21.5f64)
            .organized_by(ObjectId::ObjectsFolder)
            .insert(&mut *space);
    }

    tokio::spawn(server.run_with(listener));
    TestServer { handle, port, pki }
}

/// A connector configuration in `dir` (relative paths resolve against it) with one device
/// `plc`, whose `protocol_address` is `address` plus the endpoint, and a `[connection]` table.
fn config(
    dir: &Path,
    port: u16,
    connection: &str,
    address: &str,
) -> tedge_dot_sdk::ConnectorConfig {
    let address = if address.is_empty() {
        String::new()
    } else {
        format!(", {address}")
    };
    let text = format!(
        r#"
[connector]
protocol = "opcua"

[connection]
application_uri = "urn:tedge-dot"
connect_timeout_s = 10
{connection}

[[device]]
name = "plc"
protocol_address = {{ endpoint = "opc.tcp://127.0.0.1:{port}/"{address} }}

  [[device.point]]
  id = "t"
  datatype = "float64"
  address = {{ node_id = "ns=2;s=T" }}
"#
    );
    library::resolve(&text, dir).expect("valid config")
}

/// A connector directory whose PKI trust lists are a copy of a genpki scenario's.
fn connector_dir(vectors: &Path, scenario: &str) -> PathBuf {
    let dir = common::tempdir("connector");
    copy_dir(
        &vectors.join("scenarios").join(scenario).join("pki"),
        &dir.join("pki"),
    );
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn connect(
    connector: &mut OpcuaConnector,
    cfg: &tedge_dot_sdk::ConnectorConfig,
) -> LinkReport {
    connector.configure(cfg).expect("configure");
    let mut reports = tokio::time::timeout(Duration::from_secs(30), connector.connect())
        .await
        .expect("connect finished in time")
        .expect("connect");
    assert_eq!(reports.len(), 1);
    reports.remove(0)
}

fn reason(report: &LinkReport) -> &str {
    report.reason.as_deref().unwrap_or("")
}

/// The point the `config()` device declares (`ns=2;s=T`).
fn point_t() -> PointRef {
    PointRef {
        id: "t".to_string(),
        mode: Mode::Typed,
        datatype: Some(DataType::Float64),
        endianness: Endianness::Big,
        word_order: WordOrder::Big,
        access: Access::Read,
        unit: None,
        transform: Default::default(),
        interval: Some(Duration::from_millis(100)),
    }
}

fn secured_connection() -> &'static str {
    r#"pki_dir = "pki"
security_policy = "Basic256Sha256"
security_mode = "sign_and_encrypt""#
}

#[tokio::test]
async fn pinned_server_connects_with_generated_certificate() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let cfg = config(&dir, server.port, secured_connection(), "");
    let mut connector = OpcuaConnector::default();

    let report = connect(&mut connector, &cfg).await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);

    let expected = common::expected(&vectors);
    let info = report.info.expect("info");
    assert_eq!(info["security_policy"], "Basic256Sha256");
    assert_eq!(info["security_mode"], "sign_and_encrypt");
    assert_eq!(info["server_certificate"], "trusted");
    assert_eq!(
        info["server_thumbprint"],
        expected["scenarios"]["pinned"]["thumbprint"]
    );
    assert_eq!(
        info["endpoint"],
        format!("opc.tcp://127.0.0.1:{}/", server.port)
    );

    // The application certificate was generated into the (relative) PKI directory.
    assert!(dir.join("pki/own/certs/cert.der").exists());
    assert!(dir.join("pki/own/private/key.pem").exists());
    connector.disconnect().await.unwrap();

    // A second connector (a restart) reuses it.
    let before = std::fs::read(dir.join("pki/own/certs/cert.der")).unwrap();
    let mut again = OpcuaConnector::default();
    assert_eq!(
        connect(&mut again, &cfg).await.status,
        LinkStatus::Connected
    );
    assert_eq!(
        std::fs::read(dir.join("pki/own/certs/cert.der")).unwrap(),
        before
    );
    again.disconnect().await.unwrap();
}

#[tokio::test]
async fn every_secure_policy_and_mode_connects() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    for (policy, mode) in [
        ("Basic256Sha256", "sign"),
        ("Basic256Sha256", "sign_and_encrypt"),
        ("Aes256_Sha256_RsaPss", "sign_and_encrypt"),
    ] {
        let dir = connector_dir(&vectors, "pinned");
        let connection = r#"pki_dir = "pki""#;
        let address = format!(r#"security_policy = "{policy}", security_mode = "{mode}""#);
        let mut connector = OpcuaConnector::default();
        let report = connect(
            &mut connector,
            &config(&dir, server.port, connection, &address),
        )
        .await;
        assert_eq!(
            report.status,
            LinkStatus::Connected,
            "{policy}/{mode}: {:?}",
            report.reason
        );
        connector.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn untrusted_server_is_quarantined_until_trusted() {
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            scenario: "untrusted",
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "untrusted");
    let cfg = config(&dir, server.port, secured_connection(), "");
    let mut connector = OpcuaConnector::default();
    let thumbprint = common::expected(&vectors)["scenarios"]["untrusted"]["thumbprint"]
        .as_str()
        .unwrap()
        .to_string();

    let report = connect(&mut connector, &cfg).await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    let text = reason(&report);
    assert!(text.starts_with(security::CERTIFICATE_UNTRUSTED), "{text}");
    assert!(
        text.contains(&thumbprint) && text.contains("sim unknown"),
        "{text}"
    );
    assert_eq!(
        report.info.as_ref().unwrap()["server_thumbprint"],
        thumbprint.as_str()
    );

    let pki = Pki::new(dir.join("pki"));
    let rejected = pki.list(Group::Rejected);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].thumbprint, thumbprint);

    // Still untrusted on the next attempt.
    let report = connector.reconnect(&"plc".to_string()).await.unwrap();
    assert_eq!(report.status, LinkStatus::Disconnected);

    // The operator trusts it; the next reconnect succeeds without a restart or reload.
    let entry = pki.find(&thumbprint[..8], &[Group::Rejected]).unwrap();
    pki.relocate(&entry, Group::Trusted).unwrap();
    let report = connector.reconnect(&"plc".to_string()).await.unwrap();
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    assert_eq!(
        report.info.as_ref().unwrap()["server_certificate"],
        "trusted"
    );
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn trust_any_accepts_an_unknown_server_without_storing_it() {
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            scenario: "untrusted",
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "untrusted");
    let cfg = config(
        &dir,
        server.port,
        secured_connection(),
        "trust_any_server_certificate = true",
    );
    let mut connector = OpcuaConnector::default();

    let report = connect(&mut connector, &cfg).await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    assert_eq!(
        report.info.as_ref().unwrap()["server_certificate"],
        "not_verified"
    );
    let pki = Pki::new(dir.join("pki"));
    assert!(pki.list(Group::Rejected).is_empty());
    assert!(pki.list(Group::Trusted).is_empty());
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn invalid_and_revoked_server_certificates_are_categorised() {
    let vectors = common::genpki(&[]);
    for (scenario, prefix) in [
        ("wrong_host", security::CERTIFICATE_INVALID),
        ("expired", security::CERTIFICATE_INVALID),
        ("revoked", security::CERTIFICATE_REVOKED),
        ("ca_no_crl", security::CERTIFICATE_REVOKED),
    ] {
        let server = start_server(
            &vectors,
            ServerOptions {
                scenario,
                ..Default::default()
            },
        )
        .await;
        let dir = connector_dir(&vectors, scenario);
        let mut connector = OpcuaConnector::default();
        let report = connect(
            &mut connector,
            &config(&dir, server.port, secured_connection(), ""),
        )
        .await;
        assert_eq!(report.status, LinkStatus::Disconnected, "{scenario}");
        assert!(
            reason(&report).starts_with(prefix),
            "{scenario}: {}",
            reason(&report)
        );
        if scenario == "ca_no_crl" {
            assert!(
                reason(&report).contains("revocation unknown"),
                "{}",
                reason(&report)
            );
        }
    }
}

#[tokio::test]
async fn ca_issued_server_certificate_is_trusted_through_its_ca() {
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            scenario: "intermediate",
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "intermediate");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, secured_connection(), ""),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn server_advertising_an_unreachable_host_is_dialled_at_the_configured_address() {
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            advertised_host: "plc-internal.invalid",
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "pinned");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, secured_connection(), ""),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn no_matching_endpoint_lists_what_the_server_offers() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let address =
        r#"security_policy = "Aes128_Sha256_RsaOaep", security_mode = "sign_and_encrypt""#;
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, r#"pki_dir = "pki""#, address),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    let text = reason(&report);
    assert!(text.starts_with(security::NO_MATCHING_ENDPOINT), "{text}");
    assert!(
        text.contains("Basic256Sha256/sign")
            && text.contains("None/none")
            && text.contains("Aes256_Sha256_RsaPss/sign_and_encrypt"),
        "{text}"
    );
}

#[tokio::test]
async fn username_from_password_file() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    std::fs::write(dir.join("password"), format!("{PASSWORD}\n")).unwrap();
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(
            &dir,
            server.port,
            secured_connection(),
            r#"user = "operator", password_file = "password""#,
        ),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn rejected_credentials_are_reported_without_the_password() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(
            &dir,
            server.port,
            secured_connection(),
            r#"user = "operator", password = "wrong-guess-123""#,
        ),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    let text = reason(&report);
    assert!(text.starts_with(security::IDENTITY_REJECTED), "{text}");
    assert!(!text.contains("wrong-guess-123"), "{text}");
    assert!(!serde_json::to_string(&report.info)
        .unwrap()
        .contains("wrong-guess-123"));
}

#[tokio::test]
async fn x509_user_identity() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let address = format!(
        r#"user_certificate = "{}", user_private_key = "{}""#,
        vectors.join("users/operator.der").display(),
        vectors.join("users/operator.key.pem").display()
    );
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, secured_connection(), &address),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn plaintext_password_is_refused_unless_allowed() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let address = format!(r#"user = "operator", password = "{PASSWORD}""#);
    let mut connector = OpcuaConnector::default();
    let report = connect(&mut connector, &config(&dir, server.port, "", &address)).await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    assert!(
        reason(&report).starts_with(security::PLAINTEXT_PASSWORD_REFUSED),
        "{}",
        reason(&report)
    );

    let address = format!(r#"{address}, allow_plaintext_password = true"#);
    let mut connector = OpcuaConnector::default();
    let report = connect(&mut connector, &config(&dir, server.port, "", &address)).await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn server_that_does_not_trust_the_connector_is_reported() {
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            trust_clients: false,
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "pinned");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, secured_connection(), ""),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    let text = reason(&report);
    // Whose distrust it is decides the category: the server certificate passed our own checks,
    // so this is the server refusing us, not us refusing the server.
    assert!(
        text.starts_with(security::APPLICATION_CERTIFICATE_REJECTED),
        "{text}"
    );
    assert!(
        !text.starts_with(security::CERTIFICATE_UNTRUSTED),
        "the server's judgement of us must not be filed as our judgement of it: {text}"
    );
    assert!(
        text.contains("this connector's application certificate")
            && text.contains("tedge-dot pki export"),
        "{text}"
    );
    // The server quarantined the connector's certificate in its own PKI directory.
    assert_eq!(Pki::new(&server.pki).list(Group::Rejected).len(), 1);
}

/// A secured session must outlive the secure channel's security token: the client renews at 75%
/// of the granted lifetime, and nothing above the transport should notice. Without renewal the
/// channel would be torn down mid-run and the reads below would fail.
#[tokio::test]
async fn session_survives_secure_channel_token_renewal() {
    const LIFETIME_MS: u32 = 2_000;
    let vectors = common::genpki(&[]);
    let server = start_server(
        &vectors,
        ServerOptions {
            max_token_lifetime_ms: Some(LIFETIME_MS),
            ..Default::default()
        },
    )
    .await;
    let dir = connector_dir(&vectors, "pinned");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, secured_connection(), ""),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);

    let device = tedge_dot_sdk::DeviceId::from("plc");
    let points = [point_t()];
    let value = |s: &tedge_dot_sdk::Sample| s.value.clone();

    let first = connector.read_points(&device, &points).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].quality, Quality::Good, "{:?}", first[0].error);

    // Three token lifetimes: the client renews at 75%, so this spans several renewals.
    let deadline = std::time::Instant::now()
        + Duration::from_millis(u64::from(LIFETIME_MS) * 3 + LIFETIME_MS as u64 / 2);
    let mut reads = 0;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let s = connector.read_points(&device, &points).await.unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].quality,
            Quality::Good,
            "read failed after {reads} reads across token renewals: {:?}",
            s[0].error
        );
        assert_eq!(value(&s[0]), value(&first[0]));
        reads += 1;
    }
    assert!(reads >= 6, "expected to span several token lifetimes");
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn unsecured_configuration_creates_no_pki_directory() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = common::tempdir("unsecured");
    let mut connector = OpcuaConnector::default();
    let report = connect(
        &mut connector,
        &config(&dir, server.port, r#"pki_dir = "pki""#, ""),
    )
    .await;
    assert_eq!(report.status, LinkStatus::Connected, "{:?}", report.reason);
    assert_eq!(report.info.as_ref().unwrap()["security_policy"], "None");
    assert!(!dir.join("pki").exists());
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn missing_application_certificate_only_affects_secured_devices() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    let text = format!(
        r#"
[connector]
protocol = "opcua"

[connection]
pki_dir = "pki"
create_certificate = false

[[device]]
name = "secured"
protocol_address = {{ endpoint = "opc.tcp://127.0.0.1:{port}/", security_policy = "Basic256Sha256" }}
  [[device.point]]
  id = "t"
  datatype = "float64"
  address = {{ node_id = "ns=2;s=T" }}

[[device]]
name = "open"
protocol_address = {{ endpoint = "opc.tcp://127.0.0.1:{port}/" }}
  [[device.point]]
  id = "t"
  datatype = "float64"
  address = {{ node_id = "ns=2;s=T" }}
"#,
        port = server.port
    );
    let cfg = library::resolve(&text, &dir).unwrap();
    let mut connector = OpcuaConnector::default();
    connector
        .configure(&cfg)
        .expect("a missing certificate is not a configuration error");
    let reports = connector.connect().await.unwrap();
    let by_name = |n: &str| reports.iter().find(|r| r.device == n).unwrap();
    assert_eq!(by_name("open").status, LinkStatus::Connected);
    assert_eq!(by_name("secured").status, LinkStatus::Disconnected);
    let text = by_name("secured").reason.clone().unwrap_or_default();
    assert!(
        text.starts_with(security::APPLICATION_CERTIFICATE),
        "{text}"
    );
    assert!(text.contains("create_certificate"), "{text}");
    connector.disconnect().await.unwrap();
}

#[tokio::test]
async fn expired_application_certificate_disconnects_secured_devices() {
    let vectors = common::genpki(&[]);
    let server = start_server(&vectors, ServerOptions::default()).await;
    let dir = connector_dir(&vectors, "pinned");
    // The "expired" scenario's server certificate/key, used as the connector's own.
    let connection = format!(
        r#"{}
application_uri = "{SERVER_URI}"
certificate = "{}"
private_key = "{}""#,
        secured_connection(),
        vectors.join("scenarios/expired/server/cert.der").display(),
        vectors.join("scenarios/expired/server/key.pem").display()
    );
    let text = format!(
        r#"
[connector]
protocol = "opcua"

[connection]
{connection}

[[device]]
name = "plc"
protocol_address = {{ endpoint = "opc.tcp://127.0.0.1:{}/" }}
  [[device.point]]
  id = "t"
  datatype = "float64"
  address = {{ node_id = "ns=2;s=T" }}
"#,
        server.port
    );
    let cfg = library::resolve(&text, &dir).unwrap();
    let mut connector = OpcuaConnector::default();
    let report = connect(&mut connector, &cfg).await;
    assert_eq!(report.status, LinkStatus::Disconnected);
    let text = reason(&report);
    assert!(
        text.starts_with(security::APPLICATION_CERTIFICATE) && text.contains("expired"),
        "{text}"
    );
}
