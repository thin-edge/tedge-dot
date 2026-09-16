//! Protocol-specific configuration (spec §3).
//!
//! Fills the contract's opaque slots: `connection` (where notifications are received, the
//! defaults for requests), `device.protocol_address` (the equipment: address, version,
//! credentials) and `point.address` (an object to poll, or a notification to match). Unknown
//! keys are rejected in all three, so a misspelt `comunity` fails the load instead of silently
//! accepting every trap.
//!
//! Secrets live in [`Secret`], which has no `Display` and a redacted `Debug`, and are read from
//! a file when `*_password_file` is used. Nothing here ever puts one in an error message.

use crate::value::Oid;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use snmp2::v3::{Auth, AuthProtocol, Cipher, KeyExtension, Security, SecurityLevel};
use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;
use tedge_dot_sdk::{parse_duration, Access, DataType, Mode, PointConfig, Transform};

/// Where notifications are received when `[connection] listen` is not set.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:162";
/// The agent port when `port` is not set.
pub const DEFAULT_PORT: u16 = 161;
/// The community used for requests when none is configured.
pub const DEFAULT_COMMUNITY: &str = "public";
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_RETRIES: u32 = 1;
pub const DEFAULT_MAX_VARBINDS: usize = 20;
/// Shortest USM password, as net-snmp requires (RFC 3414 §11.2).
pub const MIN_PASSWORD_LEN: usize = 8;

// ─── secrets ─────────────────────────────────────────────────────────────────────────────────

/// A USM password. Its `Debug` is redacted and it has no `Display`, so it cannot reach a log
/// line or an error message by accident.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Secret {
        Secret(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What a password field says when it is not a string. No value is ever quoted.
const WRONG_TYPE: &str = "a password must be a password string (or use *_password_file)";

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Secret, D::Error> {
        struct SecretVisitor;
        impl<'de> Visitor<'de> for SecretVisitor {
            type Value = Secret;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a password string")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Secret, E> {
                Ok(Secret(value.as_bytes().to_vec()))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_none<E: de::Error>(self) -> Result<Secret, E> {
                Err(E::custom(WRONG_TYPE))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, _: A) -> Result<Secret, A::Error> {
                Err(de::Error::custom(WRONG_TYPE))
            }
            fn visit_map<A: de::MapAccess<'de>>(self, _: A) -> Result<Secret, A::Error> {
                Err(de::Error::custom(WRONG_TYPE))
            }
        }
        // Every non-string is refused by this visitor rather than by serde's own `invalid_type`,
        // whose message would quote the value — which for a password is the one thing that must
        // never reach a log line.
        deserializer.deserialize_any(SecretVisitor)
    }
}

// ─── the configuration as written ────────────────────────────────────────────────────────────

/// A string, or a list of strings.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(list) => list,
        }
    }
}

/// `[connection]` as written.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConnection {
    #[serde(default)]
    pub listen: Option<String>,
    #[serde(default)]
    pub community: Option<OneOrMany>,
    #[serde(default)]
    pub forwarders: Option<Vec<String>>,
    #[serde(default)]
    pub engine_id: Option<String>,
    #[serde(default)]
    pub request_timeout: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub max_varbinds: Option<usize>,
}

/// `device.protocol_address` as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDevice {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub community: Option<OneOrMany>,
    #[serde(default)]
    pub write_community: Option<String>,
    #[serde(default)]
    pub v3: Option<RawV3>,
    #[serde(default)]
    pub request_timeout: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub max_varbinds: Option<usize>,
    #[serde(default)]
    pub bulk: Option<bool>,
}

/// `device.protocol_address.v3` as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawV3 {
    pub user: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub auth_protocol: Option<String>,
    #[serde(default)]
    pub auth_password: Option<Secret>,
    #[serde(default)]
    pub auth_password_file: Option<String>,
    #[serde(default)]
    pub priv_protocol: Option<String>,
    #[serde(default)]
    pub priv_password: Option<Secret>,
    #[serde(default)]
    pub priv_password_file: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub engine_id: Option<String>,
}

/// `point.address` as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPointAddress {
    #[serde(default)]
    pub trap: Option<OneOrMany>,
    #[serde(default)]
    pub oid: Option<String>,
    #[serde(default, rename = "type")]
    pub snmp_type: Option<String>,
}

// ─── the validated model ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnmpVersion {
    V1,
    V2c,
    V3,
}

impl SnmpVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            SnmpVersion::V1 => "v1",
            SnmpVersion::V2c => "v2c",
            SnmpVersion::V3 => "v3",
        }
    }

    fn parse(text: &str) -> Result<SnmpVersion, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "v1" | "1" => Ok(SnmpVersion::V1),
            "v2c" | "2c" | "2" => Ok(SnmpVersion::V2c),
            "v3" | "3" => Ok(SnmpVersion::V3),
            other => Err(format!("version '{other}' is not \"v1\", \"v2c\" or \"v3\"")),
        }
    }
}

/// The SNMP type a SET writes (spec §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnmpType {
    Integer,
    Unsigned32,
    Gauge32,
    Counter32,
    Counter64,
    TimeTicks,
    OctetString,
    IpAddress,
    Oid,
}

impl SnmpType {
    pub fn parse(text: &str) -> Result<SnmpType, String> {
        Ok(match text.trim().to_ascii_lowercase().as_str() {
            "integer" => SnmpType::Integer,
            "unsigned32" => SnmpType::Unsigned32,
            "gauge32" => SnmpType::Gauge32,
            "counter32" => SnmpType::Counter32,
            "counter64" => SnmpType::Counter64,
            "timeticks" => SnmpType::TimeTicks,
            "octet_string" => SnmpType::OctetString,
            "ip_address" => SnmpType::IpAddress,
            "oid" => SnmpType::Oid,
            other => {
                return Err(format!(
                    "type '{other}' is not one of integer, unsigned32, gauge32, counter32, \
                     counter64, timeticks, octet_string, ip_address, oid"
                ))
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            SnmpType::Integer => "integer",
            SnmpType::Unsigned32 => "unsigned32",
            SnmpType::Gauge32 => "gauge32",
            SnmpType::Counter32 => "counter32",
            SnmpType::Counter64 => "counter64",
            SnmpType::TimeTicks => "timeticks",
            SnmpType::OctetString => "octet_string",
            SnmpType::IpAddress => "ip_address",
            SnmpType::Oid => "oid",
        }
    }

    /// The BER tag, for a `raw` write (the hex is the type's content octets).
    pub fn tag(self) -> u8 {
        match self {
            SnmpType::Integer => 0x02,
            SnmpType::OctetString => 0x04,
            SnmpType::Oid => 0x06,
            SnmpType::IpAddress => 0x40,
            SnmpType::Counter32 => 0x41,
            SnmpType::Unsigned32 | SnmpType::Gauge32 => 0x42,
            SnmpType::TimeTicks => 0x43,
            SnmpType::Counter64 => 0x46,
        }
    }

    /// The type a datatype writes when `type` is not given (spec §3.3); floats have none.
    pub fn from_datatype(datatype: DataType) -> Option<SnmpType> {
        Some(match datatype {
            DataType::Bool | DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
                SnmpType::Integer
            }
            DataType::Uint8 | DataType::Uint16 | DataType::Uint32 => SnmpType::Unsigned32,
            DataType::Uint64 => SnmpType::Counter64,
            DataType::String => SnmpType::OctetString,
            DataType::Float32 | DataType::Float64 | DataType::Bytes => return None,
        })
    }
}

/// What a point stands for (spec §3.3).
#[derive(Clone, Debug)]
pub enum PointKind {
    /// Polled with GET/GETBULK, written with SET.
    Object { oid: Oid, snmp_type: Option<SnmpType> },
    /// A notification, matched by its trap OID.
    Trap { traps: Vec<Oid> },
    /// A varbind of a notification.
    Varbind { traps: Vec<Oid>, oid: Oid },
}

#[derive(Clone, Debug)]
pub struct Point {
    pub id: String,
    pub kind: PointKind,
    pub mode: Mode,
    pub datatype: Option<DataType>,
    pub unit: Option<String>,
    pub transform: Transform,
    pub access: Access,
}

impl Point {
    /// Notification points are delivered by push; object points are polled ([`crate::SnmpConnector::pushes_point`]).
    pub fn is_pushed(&self) -> bool {
        !matches!(self.kind, PointKind::Object { .. })
    }

    /// The trap OIDs this point matches, for a notification point.
    pub fn traps(&self) -> &[Oid] {
        match &self.kind {
            PointKind::Trap { traps } | PointKind::Varbind { traps, .. } => traps,
            PointKind::Object { .. } => &[],
        }
    }

    /// The varbind OID of a varbind point.
    pub fn varbind_oid(&self) -> Option<&Oid> {
        match &self.kind {
            PointKind::Varbind { oid, .. } => Some(oid),
            _ => None,
        }
    }

    pub fn object_oid(&self) -> Option<&Oid> {
        match &self.kind {
            PointKind::Object { oid, .. } => Some(oid),
            _ => None,
        }
    }

    pub fn snmp_type(&self) -> Option<SnmpType> {
        match &self.kind {
            PointKind::Object { snmp_type, .. } => *snmp_type,
            _ => None,
        }
    }
}

/// A device's SNMPv3 credentials.
#[derive(Clone, Debug)]
pub struct V3 {
    pub user: Vec<u8>,
    pub level: SecurityLevel,
    pub auth_protocol: AuthProtocol,
    pub auth_password: Secret,
    pub cipher: Option<Cipher>,
    pub priv_password: Secret,
    pub context: String,
    /// The device's own engine ID, for requests and for accepting its traps.
    pub engine_id: Option<Vec<u8>>,
}

impl V3 {
    /// The library's security state for this user, with no engine ID yet (requests discover it,
    /// notifications carry it).
    pub fn security(&self) -> Security {
        let mut security = Security::new(&self.user, self.auth_password.as_bytes())
            .with_auth_protocol(self.auth_protocol)
            .with_auth(match (self.level, self.cipher) {
                (SecurityLevel::NoAuthNoPriv, _) => Auth::NoAuthNoPriv,
                (SecurityLevel::AuthPriv, Some(cipher)) => Auth::AuthPriv {
                    cipher,
                    privacy_password: self.priv_password.as_bytes().to_vec(),
                },
                _ => Auth::AuthNoPriv,
            });
        if matches!(self.cipher, Some(Cipher::Aes192 | Cipher::Aes256)) {
            // Needed for the short keys of MD5/SHA-1 (and SHA-224 with AES-256): the extension
            // net-snmp and pysnmp call "Blumenthal", the common one.
            security = security.with_key_extension_method(KeyExtension::Blumenthal);
        }
        if !self.context.is_empty() {
            security = security.with_context_name(&self.context);
        }
        security
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub version: SnmpVersion,
    /// Community for GET/GETBULK (v1/v2c).
    pub community: Vec<u8>,
    /// Community for SET (v1/v2c).
    pub write_community: Vec<u8>,
    /// Communities accepted on notifications; `None` accepts any.
    pub accepted: Option<Vec<Vec<u8>>>,
    pub v3: Option<V3>,
    pub request_timeout: Duration,
    pub retries: u32,
    pub max_varbinds: usize,
    pub bulk: bool,
    pub points: Vec<Point>,
}

impl Device {
    pub fn has_notifications(&self) -> bool {
        self.points.iter().any(Point::is_pushed)
    }

    pub fn has_objects(&self) -> bool {
        self.points.iter().any(|p| !p.is_pushed())
    }
}

/// `[connection]`, validated.
#[derive(Clone, Debug)]
pub struct Connection {
    pub listen: SocketAddr,
    pub communities: Option<Vec<Vec<u8>>>,
    pub forwarders: Vec<String>,
    pub engine_id: Vec<u8>,
    pub request_timeout: Duration,
    pub retries: u32,
    pub max_varbinds: usize,
}

// ─── parsing ─────────────────────────────────────────────────────────────────────────────────

pub fn connection(value: &serde_json::Value) -> Result<Connection, String> {
    let raw: RawConnection = if value.is_null() {
        RawConnection::default()
    } else {
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?
    };
    let listen = parse_listen(raw.listen.as_deref().unwrap_or(DEFAULT_LISTEN))
        .map_err(|e| format!("listen: {e}"))?;
    let communities = communities(raw.community).map_err(|e| format!("community: {e}"))?;
    let forwarders: Vec<String> = raw
        .forwarders
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.trim().to_string())
        .collect();
    if forwarders.iter().any(String::is_empty) {
        return Err("forwarders: an entry is empty".into());
    }
    let engine_id = match raw.engine_id {
        Some(text) => engine_id(&text).map_err(|e| format!("engine_id: {e}"))?,
        None => default_engine_id(),
    };
    Ok(Connection {
        listen,
        communities,
        forwarders,
        engine_id,
        request_timeout: duration(raw.request_timeout.as_deref(), DEFAULT_REQUEST_TIMEOUT)
            .map_err(|e| format!("request_timeout: {e}"))?,
        retries: raw.retries.unwrap_or(DEFAULT_RETRIES),
        max_varbinds: max_varbinds(raw.max_varbinds, DEFAULT_MAX_VARBINDS)?,
    })
}

/// One device's `protocol_address`, with `[connection]` for the defaults.
pub fn device(
    name: &str,
    address: &serde_json::Value,
    defaults: &Connection,
    points: Vec<Point>,
) -> Result<Device, String> {
    let raw: RawDevice =
        serde_json::from_value(address.clone()).map_err(|e| format!("protocol_address: {e}"))?;
    let host = raw.host.trim().to_string();
    if host.is_empty() {
        return Err("protocol_address.host is empty".into());
    }
    let version = match raw.version.as_deref() {
        Some(text) => SnmpVersion::parse(text).map_err(|e| format!("protocol_address: {e}"))?,
        None => SnmpVersion::V2c,
    };
    let accepted = match raw.community.clone() {
        Some(list) => {
            communities(Some(list)).map_err(|e| format!("protocol_address.community: {e}"))?
        }
        None => defaults.communities.clone(),
    };
    let community = raw
        .community
        .and_then(|c| c.into_vec().into_iter().next())
        .unwrap_or_else(|| DEFAULT_COMMUNITY.to_string());
    let write_community = raw.write_community.unwrap_or_else(|| community.clone());
    let v3 = match raw.v3 {
        Some(raw) => Some(v3(raw).map_err(|e| format!("protocol_address.v3: {e}"))?),
        None => None,
    };
    if version == SnmpVersion::V3 && v3.is_none() {
        return Err("protocol_address: version \"v3\" needs a `v3` table".into());
    }
    Ok(Device {
        name: name.to_string(),
        host,
        port: raw.port.unwrap_or(DEFAULT_PORT),
        version,
        community: community.into_bytes(),
        write_community: write_community.into_bytes(),
        accepted,
        v3,
        request_timeout: duration(raw.request_timeout.as_deref(), defaults.request_timeout)
            .map_err(|e| format!("protocol_address.request_timeout: {e}"))?,
        retries: raw.retries.unwrap_or(defaults.retries),
        max_varbinds: max_varbinds(raw.max_varbinds, defaults.max_varbinds)?,
        // GETBULK exists in v2c and v3 only.
        bulk: raw.bulk.unwrap_or(true) && version != SnmpVersion::V1,
        points,
    })
}

fn v3(raw: RawV3) -> Result<V3, String> {
    let user = raw.user.trim().to_string();
    if user.is_empty() {
        return Err("user is empty".into());
    }
    let auth_password = password("auth", raw.auth_password, raw.auth_password_file)?;
    let priv_password = password("priv", raw.priv_password, raw.priv_password_file)?;
    let level = match raw.level.as_deref() {
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "noauthnopriv" => SecurityLevel::NoAuthNoPriv,
            "authnopriv" => SecurityLevel::AuthNoPriv,
            "authpriv" => SecurityLevel::AuthPriv,
            other => {
                return Err(format!(
                    "level '{other}' is not \"noAuthNoPriv\", \"authNoPriv\" or \"authPriv\""
                ))
            }
        },
        None => match (auth_password.is_some(), priv_password.is_some()) {
            (true, true) => SecurityLevel::AuthPriv,
            (true, false) => SecurityLevel::AuthNoPriv,
            _ => SecurityLevel::NoAuthNoPriv,
        },
    };
    if level == SecurityLevel::NoAuthNoPriv && auth_password.is_some() {
        return Err("an authentication password is set, but level is \"noAuthNoPriv\"".into());
    }
    if level != SecurityLevel::AuthPriv && priv_password.is_some() {
        return Err(format!(
            "a privacy password is set, but level is \"{}\"",
            level_name(level)
        ));
    }
    let auth_protocol = match (level, raw.auth_protocol.as_deref()) {
        (SecurityLevel::NoAuthNoPriv, _) => AuthProtocol::Sha1,
        (_, Some(text)) => auth_protocol(text)?,
        (_, None) => return Err("auth_protocol is needed for an authenticated level".into()),
    };
    let cipher = match (level, raw.priv_protocol.as_deref()) {
        (SecurityLevel::AuthPriv, Some(text)) => Some(cipher(text)?),
        (SecurityLevel::AuthPriv, None) => {
            return Err("priv_protocol is needed for level \"authPriv\"".into())
        }
        (_, Some(_)) => {
            return Err(format!(
                "priv_protocol is set, but level is \"{}\"",
                level_name(level)
            ))
        }
        (_, None) => None,
    };
    let auth_password = match (level, auth_password) {
        (SecurityLevel::NoAuthNoPriv, _) => Secret::default(),
        (_, Some(password)) => password,
        (_, None) => return Err("auth_password or auth_password_file is needed".into()),
    };
    let priv_password = match (level, priv_password) {
        (SecurityLevel::AuthPriv, Some(password)) => password,
        (SecurityLevel::AuthPriv, None) => {
            return Err("priv_password or priv_password_file is needed for level \"authPriv\"".into())
        }
        _ => Secret::default(),
    };
    let engine_id = match raw.engine_id {
        Some(text) => Some(engine_id(&text).map_err(|e| format!("engine_id: {e}"))?),
        None => None,
    };
    Ok(V3 {
        user: user.into_bytes(),
        level,
        auth_protocol,
        auth_password,
        cipher,
        priv_password,
        context: raw.context.unwrap_or_default(),
        engine_id,
    })
}

fn level_name(level: SecurityLevel) -> &'static str {
    match level {
        SecurityLevel::NoAuthNoPriv => "noAuthNoPriv",
        SecurityLevel::AuthNoPriv => "authNoPriv",
        SecurityLevel::AuthPriv => "authPriv",
    }
}

fn auth_protocol(text: &str) -> Result<AuthProtocol, String> {
    Ok(match text.trim().to_ascii_uppercase().as_str() {
        "MD5" => AuthProtocol::Md5,
        "SHA" | "SHA1" => AuthProtocol::Sha1,
        "SHA224" => AuthProtocol::Sha224,
        "SHA256" => AuthProtocol::Sha256,
        "SHA384" => AuthProtocol::Sha384,
        "SHA512" => AuthProtocol::Sha512,
        other => {
            return Err(format!(
                "auth_protocol '{other}' is not MD5, SHA, SHA224, SHA256, SHA384 or SHA512"
            ))
        }
    })
}

fn cipher(text: &str) -> Result<Cipher, String> {
    Ok(match text.trim().to_ascii_uppercase().as_str() {
        "DES" => Cipher::Des,
        "AES" | "AES128" => Cipher::Aes128,
        "AES192" => Cipher::Aes192,
        "AES256" => Cipher::Aes256,
        other => {
            return Err(format!(
                "priv_protocol '{other}' is not DES, AES, AES192 or AES256"
            ))
        }
    })
}

/// Exactly one of `<what>_password` and `<what>_password_file`. A file is read here, at
/// configure: the secret never has to live in the file management commands rewrite.
fn password(what: &str, inline: Option<Secret>, file: Option<String>) -> Result<Option<Secret>, String> {
    let secret = match (inline, file) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "{what}_password and {what}_password_file are both set; use one"
            ))
        }
        (Some(secret), None) => secret,
        (None, Some(path)) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("{what}_password_file '{path}' cannot be read: {e}"))?;
            let line = text.lines().next().unwrap_or_default();
            Secret::new(line.trim_end_matches('\r').as_bytes().to_vec())
        }
        (None, None) => return Ok(None),
    };
    if secret.len() < MIN_PASSWORD_LEN {
        return Err(format!(
            "the {what} password is shorter than {MIN_PASSWORD_LEN} characters (USM's minimum)"
        ));
    }
    Ok(Some(secret))
}

/// One point (spec §3.3).
pub fn point(p: &PointConfig, default_mode: Option<Mode>) -> Result<Point, String> {
    let address: RawPointAddress =
        serde_json::from_value(p.address.clone()).map_err(|e| format!("address: {e}"))?;
    let traps = trap_oids(address.trap).map_err(|e| format!("address.trap: {e}"))?;
    let oid = address
        .oid
        .as_deref()
        .map(Oid::parse)
        .transpose()
        .map_err(|e| format!("address.oid: {e}"))?;
    let snmp_type = address
        .snmp_type
        .as_deref()
        .map(SnmpType::parse)
        .transpose()
        .map_err(|e| format!("address.{e}"))?;
    let access = Access::parse(p.access.as_deref());
    let mode = p.resolved_mode(default_mode);
    if p.datatype == Some(DataType::Bytes) {
        return Err("datatype \"bytes\" is not supported; use mode = \"raw\"".into());
    }
    if mode == Mode::Typed && p.datatype.is_none() {
        return Err("a typed point needs a datatype".into());
    }

    let kind = match (traps.is_empty(), oid) {
        // An object: polled, and written when its access allows it.
        (true, Some(oid)) => {
            let snmp_type = match (snmp_type, p.datatype) {
                (Some(snmp_type), _) => Some(snmp_type),
                (None, Some(datatype)) => SnmpType::from_datatype(datatype),
                (None, None) => None,
            };
            if access.can_write() && snmp_type.is_none() {
                return Err(
                    "a writable object point needs address.type: the SNMP type a SET writes \
                     cannot be derived from this point's datatype"
                        .into(),
                );
            }
            PointKind::Object { oid, snmp_type }
        }
        // A notification, or one of its varbinds.
        (false, oid) => {
            if snmp_type.is_some() {
                return Err("address.type applies to object points only".into());
            }
            if access != Access::Read {
                return Err(
                    "a notification point is read-only: a notification cannot be written back".into(),
                );
            }
            if p.subscribe == Some(false) {
                return Err(
                    "subscribe = false is not supported on a notification point: there is \
                     nothing to poll"
                        .into(),
                );
            }
            match oid {
                Some(oid) => PointKind::Varbind { traps, oid },
                None => {
                    if mode == Mode::Typed
                        && !matches!(p.datatype, Some(DataType::String | DataType::Bool))
                    {
                        return Err(
                            "a trap point (address.trap without address.oid) is typed \"string\" \
                             (the trap OID) or \"bool\""
                                .into(),
                        );
                    }
                    PointKind::Trap { traps }
                }
            }
        }
        (true, None) => {
            return Err(
                "address needs `oid` (an object to poll) or `trap` (a notification)".into(),
            )
        }
    };

    Ok(Point {
        id: p.id.clone(),
        kind,
        mode,
        datatype: p.datatype,
        unit: p.unit.clone(),
        transform: p.transform.unwrap_or_default(),
        access,
    })
}

/// Parse `listen`: an IP socket address, never a host name (a receiver binds an address).
pub fn parse_listen(text: &str) -> Result<SocketAddr, String> {
    text.trim()
        .parse::<SocketAddr>()
        .map_err(|_| format!("'{text}' is not <IPv4>:<port> or [<IPv6>]:<port>"))
}

/// The accepted communities as octets; `None` accepts any. An empty list would accept nothing,
/// which is never what was meant, so it is an error.
pub fn communities(field: Option<OneOrMany>) -> Result<Option<Vec<Vec<u8>>>, String> {
    let Some(field) = field else { return Ok(None) };
    let list = field.into_vec();
    if list.is_empty() {
        return Err("an empty list accepts no notification; omit the key to accept any".into());
    }
    Ok(Some(list.into_iter().map(String::into_bytes).collect()))
}

/// Parse a `trap` value into its OIDs.
pub fn trap_oids(field: Option<OneOrMany>) -> Result<Vec<Oid>, String> {
    let Some(field) = field else { return Ok(Vec::new()) };
    let list = field.into_vec();
    if list.is_empty() {
        return Err("an empty list matches no notification".into());
    }
    list.iter().map(|s| Oid::parse(s)).collect()
}

/// An engine ID as hex, with or without a leading `0x` (5 to 32 octets, RFC 3411).
pub fn engine_id(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let body = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    if body.is_empty() || !body.len().is_multiple_of(2) || !body.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!("'{text}' is not an even number of hex digits"));
    }
    let bytes: Vec<u8> = (0..body.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&body[i..i + 2], 16).expect("checked above"))
        .collect();
    if !(5..=32).contains(&bytes.len()) {
        return Err(format!("'{text}' is {} octets, not 5 to 32", bytes.len()));
    }
    Ok(bytes)
}

/// This receiver's engine ID when none is configured (spec §3.1): the RFC 3411 "text" format
/// with enterprise 0, over the machine's stable identity, so it survives restarts.
pub fn default_engine_id() -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let identity = ["/etc/machine-id", "/var/lib/dbus/machine-id", "/etc/hostname"]
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "tedge-dot".to_string());
    let digest = Sha256::digest(identity.as_bytes());
    let mut engine_id = vec![0x80, 0x00, 0x00, 0x00, 0x05];
    engine_id.extend_from_slice(&digest[..8]);
    engine_id
}

/// snmpEngineBoots for this run. Without a file to count restarts in, the wall clock stands in:
/// it grows with every restart, which is what a sender that cached our boots needs (RFC 3414
/// §2.2). Seconds since 2020-01-01, so it stays well inside 2^31 - 1.
pub fn engine_boots() -> i64 {
    const EPOCH_2020: u64 = 1_577_836_800;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(EPOCH_2020);
    i64::try_from(now.saturating_sub(EPOCH_2020))
        .unwrap_or(1)
        .clamp(1, i64::from(i32::MAX) - 1)
}

fn duration(text: Option<&str>, default: Duration) -> Result<Duration, String> {
    match text {
        None => Ok(default),
        Some(text) => parse_duration(text)
            .filter(|d| !d.is_zero())
            .ok_or_else(|| format!("'{text}' is not a duration like \"2s\" or \"500ms\"")),
    }
}

fn max_varbinds(value: Option<usize>, default: usize) -> Result<usize, String> {
    match value {
        None => Ok(default),
        Some(0) => Err("max_varbinds: a request carries at least one OID".into()),
        Some(n) => Ok(n),
    }
}

/// Addresses two devices must not share (spec §3.2), checked on the literals at configure.
pub fn check_unique_hosts(devices: &[Device], forwarders: &[String]) -> Result<(), String> {
    let mut seen: HashSet<std::net::IpAddr> = HashSet::new();
    let forwarder_ips: HashSet<std::net::IpAddr> = forwarders
        .iter()
        .filter_map(|f| f.parse::<std::net::IpAddr>().ok())
        .collect();
    for device in devices {
        let Ok(ip) = device.host.parse::<std::net::IpAddr>() else {
            continue;
        };
        if !seen.insert(ip) {
            return Err(format!(
                "device '{}': protocol_address.host {ip} is also another device's host; \
                 notifications are routed by source address, so it can belong to one device only",
                device.name
            ));
        }
        if forwarder_ips.contains(&ip) {
            return Err(format!(
                "device '{}': protocol_address.host {ip} is also a trusted forwarder",
                device.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_accepts_v4_and_bracketed_v6_only() {
        assert!(parse_listen("0.0.0.0:162").is_ok());
        assert!(parse_listen("[::]:1162").is_ok());
        assert!(parse_listen("localhost:162").is_err());
        assert!(parse_listen("0.0.0.0").is_err());
    }

    #[test]
    fn community_forms() {
        assert_eq!(communities(None).unwrap(), None);
        assert_eq!(
            communities(Some(OneOrMany::One("public".into()))).unwrap(),
            Some(vec![b"public".to_vec()])
        );
        assert!(communities(Some(OneOrMany::Many(vec![]))).is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = serde_json::from_value::<RawDevice>(
            serde_json::json!({ "host": "10.0.0.1", "comunity": "public" }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("comunity"), "{err}");
        assert!(serde_json::from_value::<RawConnection>(serde_json::json!({ "port": 162 })).is_err());
        assert!(
            serde_json::from_value::<RawPointAddress>(serde_json::json!({ "varbind": "1.3" })).is_err()
        );
        assert!(serde_json::from_value::<RawV3>(
            serde_json::json!({ "user": "u", "password": "secret-value-1" })
        )
        .is_err());
    }

    #[test]
    fn a_secret_never_reaches_a_message() {
        let secret = Secret::new(b"hunter2-hunter2".to_vec());
        assert_eq!(format!("{secret:?}"), "<redacted>");
        let v3 = RawV3 {
            user: "u".into(),
            level: None,
            auth_protocol: Some("SHA".into()),
            auth_password: Some(secret),
            auth_password_file: None,
            priv_protocol: None,
            priv_password: None,
            priv_password_file: None,
            context: None,
            engine_id: None,
        };
        assert!(!format!("{v3:?}").contains("hunter2"), "{v3:?}");
        // A password that is not a string: the message names the field, never the value.
        let err = serde_json::from_value::<RawV3>(
            serde_json::json!({ "user": "u", "auth_password": 12345678 }),
        )
        .unwrap_err()
        .to_string();
        assert!(!err.contains("12345678"), "{err}");
        assert!(err.contains("password string"), "{err}");
    }

    #[test]
    fn engine_ids_are_hex_of_five_to_thirty_two_octets() {
        assert_eq!(engine_id("0x8000000001").unwrap(), vec![0x80, 0, 0, 0, 1]);
        assert_eq!(engine_id("8000000001").unwrap(), vec![0x80, 0, 0, 0, 1]);
        for bad in ["", "80000000", "0x800000000", "zz00000000", "80000000010203"] {
            if bad == "80000000010203" {
                assert!(engine_id(bad).is_ok());
                continue;
            }
            assert!(engine_id(bad).is_err(), "{bad} should be rejected");
        }
        assert!(engine_id(&"ab".repeat(33)).is_err());
        let default = default_engine_id();
        assert_eq!(default.len(), 13);
        assert_eq!(default_engine_id(), default, "stable across calls");
        assert!(engine_boots() > 0);
    }

    #[test]
    fn a_password_comes_from_the_file_or_the_key_but_not_both() {
        let dir = std::env::temp_dir().join(format!("tdot-snmp-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth");
        std::fs::write(&path, "file-password-1\nignored\n").unwrap();
        let from_file = password("auth", None, Some(path.display().to_string())).unwrap().unwrap();
        assert_eq!(from_file.as_bytes(), b"file-password-1");
        assert!(password("auth", Some(Secret::new(b"inline-password".to_vec())), Some(path.display().to_string())).is_err());
        assert!(password("auth", Some(Secret::new(b"short".to_vec())), None).is_err(), "USM minimum");
        assert!(password("auth", None, Some("/nonexistent/secret".into())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
