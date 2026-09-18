//! In-process tests of the whole module: a real UDP socket on loopback for the notifications,
//! and a small SNMP agent (built with the same library) for the polled and written ones.

use connector_snmp::snmp2::v3::{
    self, Auth, AuthProtocol, Cipher, Header, LocalEngine, Outgoing, Security, SecurityLevel,
};
use connector_snmp::snmp2::{pdu, MessageType, Oid as LibOid, Pdu, Value as SnmpValue, Version as LibVersion};
use connector_snmp::value::Oid;
use connector_snmp::factory;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tedge_dot_sdk::{
    Access, Connector, ConnectorConfig, DeviceId, Endianness, LinkStatus, Mode, PointRef, Quality,
    Sample, Transform, Value, WordOrder,
};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

// ─── fixtures ────────────────────────────────────────────────────────────────────────────────

/// net-snmp `snmptrap -v 2c -c public ... linkDown ifIndex.3 i 3 ifAdminStatus.3 i 2 ifOperStatus.3 i 2`
const LINK_DOWN: &str = "307702010104067075626c6963a76a020479d8753a020100020100305c300e06082b06010201010300430230393017060a2b06010603010104010006092b0601060301010503300f060a2b060102010202010103020103300f060a2b060102010202010703020102300f060a2b060102010202010803020102";
/// net-snmp `snmpinform -v 2c -c public ... 1.3.6.1.4.1.99999.0.2 1.3.6.1.4.1.99999.2.1 s "inform me"`
const INFORM: &str = "305e02010104067075626c6963a651020454707b700201000201003043300e06082b06010201010300430203093018060a2b060106030101040100060a2b06010401868d1f00023017060a2b06010401868d1f02010409696e666f726d206d65";
/// net-snmp `snmptrap -v 2c -c private ...` carrying one varbind of every type.
const PRIVATE_TRAP: &str = "3081ea020101040770726976617465a781db02042fa7210d0201000201003081cc300d06082b060102010103004301003018060a2b060106030101040100060a2b06010401868d1f00013014060a2b06010401868d1f0201040668c3a96c6c6f3011060a2b06010401868d1f0203410301e240300f060a2b06010401868d1f02044301633012060a2b06010401868d1f02054004c0a801143015060a2b06010401868d1f020606072b06010401bf083012060a2b06010401868d1f02070404deadbeef300f060a2b06010401868d1f02080201d63017060a2b06010401868d1f0209460900ffffffffffffffff";
/// net-snmp `snmptrap -v 3 -e 0x8000000001020304 -u trapuser -l authPriv -a SHA -A authpassword -x AES -X privpassword ... linkDown ifIndex.3 i 3 ...`
const V3_TRAP: &str = "3081af0201033011020470f05afb020300ffe3040103020103043430320408800000000102030402010002010004087472617075736572040c050ce04e23b31f144145ce6d040851e6aaec28b3400d046185e6a5a39a4498677e1d16ef49f4bed0af373de4554b49a387e22e269c881f7e11677c9eb80b812195e9f121437b4638c6bfd46c938388be9e0904d5cd97ce5657f7cffffb7f37abc697543443bb29c37323c37e5c06807d5ff38cd2b69101ca78";
/// The same trap with the wrong authentication password.
const V3_TRAP_WRONG_KEY: &str = "3081af02010330110204675caea0020300ffe3040103020103043430320408800000000102030402010002010004087472617075736572040c18986172bf59f9b8be0f5c1e0408ef501818bc82e9c20461e3266a52844b5a841c83c40fcde9e95e36f499443acf35a97228b1f0c232c94f500290556c77c2133775f1a522a361ce46f29968286f451bdddad61e2aecb69dd73c86a095a91050161e0cb51704a4c0ac5ff129222963404b2a3b7da112fb64c4";

const TRAP_ENGINE_ID: &str = "8000000001020304";
const RECEIVER_ENGINE_ID: &str = "80000000057465737431";

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn ipv6_loopback() -> bool {
    std::net::UdpSocket::bind("[::1]:0").is_ok()
}

fn config(text: &str) -> ConnectorConfig {
    toml::from_str(text).unwrap_or_else(|e| panic!("test config does not parse: {e}\n{text}"))
}

fn refs(ids: &[&str]) -> Vec<PointRef> {
    ids.iter()
        .map(|id| PointRef {
            id: id.to_string(),
            mode: Mode::Typed,
            datatype: None,
            endianness: Endianness::Big,
            word_order: WordOrder::Big,
            access: Access::Read,
            unit: None,
            transform: Transform::default(),
            interval: None,
        })
        .collect()
}

async fn next(rx: &mut mpsc::Receiver<Sample>) -> Sample {
    tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("a sample within 3s")
        .expect("sink open")
}

async fn nothing(rx: &mut mpsc::Receiver<Sample>) {
    if let Ok(Some(s)) = tokio::time::timeout(Duration::from_millis(400), rx.recv()).await {
        panic!("expected no sample, got {} = {:?}", s.point, s.value);
    }
}

async fn subscribe(conn: &mut Box<dyn Connector>, device: &str, ids: &[&str]) -> mpsc::Receiver<Sample> {
    let (tx, rx) = mpsc::channel(64);
    conn.subscribe(&DeviceId::from(device), &refs(ids), tx).await.expect("subscribe");
    rx
}

async fn read(conn: &mut Box<dyn Connector>, device: &str, ids: &[&str]) -> Vec<Sample> {
    conn.read_points(&DeviceId::from(device), &refs(ids)).await.expect("read_points")
}

fn sample<'a>(samples: &'a [Sample], id: &str) -> &'a Sample {
    samples
        .iter()
        .find(|s| s.point == id)
        .unwrap_or_else(|| panic!("no sample for '{id}' in {:?}", samples.iter().map(|s| &s.point).collect::<Vec<_>>()))
}

// ─── a small SNMP agent, built with the same library ─────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Owned {
    Int(i64),
    Str(Vec<u8>),
    Counter32(u32),
    Gauge(u32),
    Ticks(u32),
    Counter64(u64),
    Ip([u8; 4]),
    Oid(Vec<u8>),
    NoSuchObject,
    EndOfMibView,
}

impl Owned {
    fn value(&self) -> SnmpValue<'_> {
        match self {
            Owned::Int(v) => SnmpValue::Integer(*v),
            Owned::Str(b) => SnmpValue::OctetString(b),
            Owned::Counter32(v) => SnmpValue::Counter32(*v),
            Owned::Gauge(v) => SnmpValue::Unsigned32(*v),
            Owned::Ticks(v) => SnmpValue::Timeticks(*v),
            Owned::Counter64(v) => SnmpValue::Counter64(*v),
            Owned::Ip(ip) => SnmpValue::IpAddress(*ip),
            Owned::Oid(bytes) => SnmpValue::ObjectIdentifier(LibOid::new(std::borrow::Cow::Borrowed(bytes))),
            Owned::NoSuchObject => SnmpValue::NoSuchObject,
            Owned::EndOfMibView => SnmpValue::EndOfMibView,
        }
    }

    fn of(value: &SnmpValue<'_>) -> Option<Owned> {
        Some(match value {
            SnmpValue::Integer(v) => Owned::Int(*v),
            SnmpValue::OctetString(b) => Owned::Str(b.to_vec()),
            SnmpValue::Counter32(v) => Owned::Counter32(*v),
            SnmpValue::Unsigned32(v) => Owned::Gauge(*v),
            SnmpValue::Timeticks(v) => Owned::Ticks(*v),
            SnmpValue::Counter64(v) => Owned::Counter64(*v),
            SnmpValue::IpAddress(ip) => Owned::Ip(*ip),
            SnmpValue::ObjectIdentifier(oid) => Owned::Oid(oid.as_bytes().to_vec()),
            _ => return None,
        })
    }
}

struct Agent {
    objects: Vec<(Oid, Owned)>,
    read_only: HashSet<String>,
    log: Vec<String>,
    silent: bool,
    community: Vec<u8>,
    v1: bool,
    v3: Option<(LocalEngine, Security)>,
}

impl Agent {
    fn seeded() -> Agent {
        let mut objects = vec![
            ("1.3.6.1.2.1.1.1.0", Owned::Str(b"tedge-dot test agent".to_vec())),
            ("1.3.6.1.4.1.99999.1.1.0", Owned::Int(-42)),
            ("1.3.6.1.4.1.99999.1.3.0", Owned::Str("Zürich".as_bytes().to_vec())),
            ("1.3.6.1.4.1.99999.1.7.0", Owned::Counter32(4_000_000_000)),
            ("1.3.6.1.4.1.99999.1.10.0", Owned::Counter64(9_007_199_254_740_993)),
            ("1.3.6.1.4.1.99999.1.6.0", Owned::Ip([192, 168, 10, 2])),
            // A table cell: not a scalar, so it cannot be fetched with GETBULK.
            ("1.3.6.1.4.1.99999.2.1.1", Owned::Int(7)),
            ("1.3.6.1.4.1.99999.1.20.0", Owned::Int(10)),
            ("1.3.6.1.4.1.99999.1.21.0", Owned::Str(b"initial".to_vec())),
            ("1.3.6.1.4.1.99999.1.22.0", Owned::Counter32(100)),
        ]
        .into_iter()
        .map(|(oid, value)| (Oid::parse(oid).unwrap(), value))
        .collect::<Vec<_>>();
        objects.sort_by(|a, b| a.0.cmp(&b.0));
        Agent {
            objects,
            read_only: ["1.3.6.1.4.1.99999.1.1.0".to_string()].into_iter().collect(),
            log: Vec::new(),
            silent: false,
            community: b"public".to_vec(),
            v1: false,
            v3: None,
        }
    }

    fn get(&self, oid: &Oid) -> Option<&Owned> {
        self.objects.iter().find(|(name, _)| name == oid).map(|(_, value)| value)
    }

    /// Answer one datagram, or nothing when the agent is playing dead.
    fn answer(&mut self, datagram: &[u8]) -> Option<Vec<u8>> {
        if self.silent {
            self.log.push("ignored".into());
            return None;
        }
        if Header::parse(datagram).is_ok() {
            return self.answer_v3(datagram);
        }
        let request = Pdu::from_bytes(datagram).ok()?;
        if request.community != self.community {
            return None;
        }
        self.log.push(format!("{:?}", request.message_type));
        let version = request.version().ok()?;
        let (error_status, error_index, varbinds) = answer_pdu(
            &mut self.objects,
            &self.read_only,
            &request,
            self.v1,
            version == LibVersion::V1,
        );
        let names: Vec<LibOid<'static>> = varbinds.iter().map(|(oid, _)| oid.to_library()).collect();
        let values: Vec<(&LibOid<'static>, SnmpValue)> = names
            .iter()
            .zip(varbinds.iter())
            .map(|(name, (_, value))| (name, value.value()))
            .collect();
        pdu::encode(
            version,
            &self.community,
            MessageType::Response,
            request.req_id,
            error_status,
            error_index,
            &values,
        )
        .ok()
    }

    fn answer_v3(&mut self, datagram: &[u8]) -> Option<Vec<u8>> {
        let (engine, user) = self.v3.as_mut()?;
        match engine.receive(datagram, Some(user)) {
            Ok(message) => {
                self.log.push(format!("v3 {:?}", message.pdu.message_type));
                let (error_status, error_index, varbinds) =
                    answer_pdu(&mut self.objects, &self.read_only, &message.pdu, false, false);
                let names: Vec<LibOid<'static>> =
                    varbinds.iter().map(|(oid, _)| oid.to_library()).collect();
                let values: Vec<(&LibOid<'static>, SnmpValue)> = names
                    .iter()
                    .zip(varbinds.iter())
                    .map(|(name, (_, value))| (name, value.value()))
                    .collect();
                engine
                    .respond(&message, MessageType::Response, error_status, error_index, &values)
                    .ok()
            }
            Err(refusal) => {
                self.log.push(format!("v3 refused: {}", refusal.error));
                refusal.report
            }
        }
    }
}

/// The varbinds one request is answered with (GET, GETBULK's non-repeaters, SET).
fn answer_pdu(
    objects: &mut [(Oid, Owned)],
    read_only: &HashSet<String>,
    request: &Pdu<'_>,
    _v1_agent: bool,
    v1: bool,
) -> (u32, u32, Vec<(Oid, Owned)>) {
    let requested: Vec<(Oid, Option<Owned>)> = request
        .varbinds
        .clone()
        .map(|(name, value)| {
            (Oid::from_library(&name).expect("a valid OID"), Owned::of(&value))
        })
        .collect();
    let mut out = Vec::with_capacity(requested.len());
    match request.message_type {
        MessageType::GetRequest => {
            for (index, (oid, _)) in requested.iter().enumerate() {
                match objects.iter().find(|(name, _)| name == oid) {
                    Some((_, value)) => out.push((oid.clone(), value.clone())),
                    // SNMPv1 has no exceptions: the whole request fails, naming the varbind.
                    None if v1 => return (2, index as u32 + 1, Vec::new()),
                    None => out.push((oid.clone(), Owned::NoSuchObject)),
                }
            }
        }
        MessageType::GetBulkRequest => {
            for (oid, _) in &requested {
                match objects.iter().find(|(name, _)| name > oid).cloned() {
                    Some((name, value)) => out.push((name, value)),
                    None => out.push((oid.clone(), Owned::EndOfMibView)),
                }
            }
        }
        MessageType::SetRequest => {
            for (index, (oid, value)) in requested.iter().enumerate() {
                let Some(value) = value.clone() else {
                    return (7, index as u32 + 1, Vec::new()); // wrongType
                };
                if read_only.contains(&oid.to_string()) {
                    return (17, index as u32 + 1, Vec::new()); // notWritable
                }
                match objects.iter_mut().find(|(name, _)| name == oid) {
                    Some(entry) => entry.1 = value.clone(),
                    None => return (17, index as u32 + 1, Vec::new()),
                }
                out.push((oid.clone(), value));
            }
        }
        _ => return (5, 0, Vec::new()), // genErr
    }
    (0, 0, out)
}

#[derive(Clone)]
struct AgentHandle {
    address: SocketAddr,
    state: Arc<Mutex<Agent>>,
}

impl AgentHandle {
    fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    fn value(&self, oid: &str) -> Option<Owned> {
        let oid = Oid::parse(oid).unwrap();
        self.state.lock().unwrap().get(&oid).cloned()
    }
}

async fn spawn_agent(agent: Agent) -> AgentHandle {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let state = Arc::new(Mutex::new(agent));
    let task_state = state.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let Ok((len, peer)) = socket.recv_from(&mut buf).await else { continue };
            let answer = task_state.lock().unwrap().answer(&buf[..len]);
            if let Some(answer) = answer {
                let _ = socket.send_to(&answer, peer).await;
            }
        }
    });
    AgentHandle { address, state }
}

fn v3_agent_security() -> Security {
    Security::new(b"rw-priv", b"auth-password-1")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthPriv {
            cipher: Cipher::Aes128,
            privacy_password: b"priv-password-1".to_vec(),
        })
}

// ─── polling and writes ──────────────────────────────────────────────────────────────────────

fn agent_config(port: u16, extra: &str) -> String {
    versioned_agent_config(port, "v2c", extra)
}

fn versioned_agent_config(port: u16, version: &str, extra: &str) -> String {
    format!(
        r#"
[connector]
protocol = "snmp"

[connection]
listen = "127.0.0.1:{listen}"
request_timeout = "500ms"
retries = 0

[[device]]
name             = "agent"
protocol_address = {{ host = "127.0.0.1", port = {port}, version = "{version}", community = "public", write_community = "public" }}
default_mode     = "typed"
{extra}
"#,
        listen = free_port(),
    )
}

const AGENT_POINTS: &str = r#"
  [[device.point]]
  id       = "sys_descr"
  datatype = "string"
  address  = { oid = "1.3.6.1.2.1.1.1.0" }

  [[device.point]]
  id       = "negative"
  datatype = "int32"
  address  = { oid = "1.3.6.1.4.1.99999.1.1.0" }

  [[device.point]]
  id       = "counter32"
  datatype = "uint32"
  address  = { oid = "1.3.6.1.4.1.99999.1.7.0" }

  [[device.point]]
  id       = "counter64"
  datatype = "uint64"
  address  = { oid = "1.3.6.1.4.1.99999.1.10.0" }

  [[device.point]]
  id       = "ip"
  datatype = "string"
  address  = { oid = "1.3.6.1.4.1.99999.1.6.0" }

  [[device.point]]
  id       = "cell"
  datatype = "int32"
  address  = { oid = "1.3.6.1.4.1.99999.2.1.1" }

  [[device.point]]
  id       = "missing"
  datatype = "int32"
  address  = { oid = "1.3.6.1.4.1.99999.1.99.0" }

  [[device.point]]
  id       = "utf8"
  datatype = "string"
  address  = { oid = "1.3.6.1.4.1.99999.1.3.0" }
"#;

#[tokio::test]
async fn polled_objects_use_getbulk_for_scalars_and_get_for_the_rest() {
    let agent = spawn_agent(Agent::seeded()).await;
    let mut conn = factory();
    conn.configure(&config(&agent_config(agent.address.port(), AGENT_POINTS))).unwrap();
    let reports = conn.connect().await.unwrap();
    assert_eq!(reports[0].status, LinkStatus::Connected, "{:?}", reports[0].reason);
    assert_eq!(
        reports[0].info,
        Some(serde_json::json!({ "host": "127.0.0.1", "port": agent.address.port(), "version": "v2c" })),
        "link info is host, port and version only"
    );

    let samples = read(&mut conn, "agent", &["sys_descr", "negative", "counter32", "counter64", "ip", "cell", "utf8"]).await;
    assert_eq!(samples.len(), 7);
    assert_eq!(sample(&samples, "sys_descr").value, Some(Value::Text("tedge-dot test agent".into())));
    assert_eq!(sample(&samples, "sys_descr").addr["oid"], "1.3.6.1.2.1.1.1.0");
    assert_eq!(sample(&samples, "negative").value, Some(Value::Number(-42.0)));
    assert_eq!(sample(&samples, "counter32").value, Some(Value::Number(4_000_000_000.0)));
    // Beyond 2^53 a number would lose digits, so it is published as a decimal string (§6).
    assert_eq!(sample(&samples, "counter64").value, Some(Value::Text("9007199254740993".into())));
    assert_eq!(sample(&samples, "ip").value, Some(Value::Text("192.168.10.2".into())));
    assert_eq!(sample(&samples, "utf8").value, Some(Value::Text("Zürich".into())));
    assert_eq!(sample(&samples, "cell").value, Some(Value::Number(7.0)), "a table cell is fetched with GET");
    assert!(samples.iter().all(|s| s.quality == Quality::Good), "{samples:?}");

    let log = agent.log();
    assert!(log.contains(&"GetBulkRequest".to_string()), "scalars go in a GETBULK: {log:?}");
    assert!(log.contains(&"GetRequest".to_string()), "a non-scalar instance goes in a GET: {log:?}");
}

#[tokio::test]
async fn exceptions_and_wrong_names_are_bad_samples() {
    let agent = spawn_agent(Agent::seeded()).await;
    let mut conn = factory();
    conn.configure(&config(&agent_config(agent.address.port(), AGENT_POINTS))).unwrap();
    conn.connect().await.unwrap();

    let samples = read(&mut conn, "agent", &["missing", "sys_descr"]).await;
    let missing = sample(&samples, "missing");
    assert_eq!(missing.quality, Quality::Bad);
    assert!(missing.raw.is_empty(), "an exception carries no octets");
    assert!(missing.error.as_deref().unwrap_or("").contains("1.3.6.1.4.1.99999.1.99"), "{missing:?}");
    assert_eq!(sample(&samples, "sys_descr").quality, Quality::Good, "the good ones are unaffected");
}

#[tokio::test]
async fn a_v1_agent_fails_only_the_point_at_the_error_index() {
    let mut agent = Agent::seeded();
    agent.v1 = true;
    let agent = spawn_agent(agent).await;
    let text = versioned_agent_config(agent.address.port(), "v1", AGENT_POINTS);
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();

    let samples = read(&mut conn, "agent", &["sys_descr", "missing", "negative"]).await;
    assert_eq!(samples.len(), 3);
    let missing = sample(&samples, "missing");
    assert_eq!(missing.quality, Quality::Bad);
    assert!(missing.error.as_deref().unwrap_or("").contains("noSuchName"), "{missing:?}");
    assert_eq!(sample(&samples, "sys_descr").quality, Quality::Good, "re-requested without it");
    assert_eq!(sample(&samples, "negative").value, Some(Value::Number(-42.0)));
    assert!(
        !agent.log().contains(&"GetBulkRequest".to_string()),
        "SNMPv1 has no GETBULK: {:?}",
        agent.log()
    );
}

#[tokio::test]
async fn a_silent_agent_makes_the_batch_bad_and_skips_the_rest() {
    let mut agent = Agent::seeded();
    agent.silent = true;
    let agent = spawn_agent(agent).await;
    // Two batches arise from the point kinds, not from a varbind cap: `cell` is a non-scalar, so
    // it goes in its own GET beside the scalars' GETBULK (see
    // `polled_objects_use_getbulk_for_scalars_and_get_for_the_rest`). An earlier
    // `.replace("max_varbinds", "unused")` here edited nothing -- the string does not occur in
    // this config -- so it only made the split look configured.
    let text = agent_config(agent.address.port(), AGENT_POINTS);
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();

    let started = std::time::Instant::now();
    let samples = read(&mut conn, "agent", &["sys_descr", "negative", "cell"]).await;
    assert_eq!(samples.len(), 3);
    assert!(samples.iter().all(|s| s.quality == Quality::Bad), "{samples:?}");
    assert!(
        samples.iter().any(|s| s.error.as_deref().unwrap_or("").contains("no answer within")),
        "{samples:?}"
    );
    assert!(
        samples.iter().any(|s| s.error.as_deref().unwrap_or("").starts_with("skipped")),
        "the second batch is not waited for as well: {samples:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2), "one timeout, not one per batch");
}

#[tokio::test]
async fn writes_go_through_with_the_configured_type_and_errors_carry_their_name() {
    let agent = spawn_agent(Agent::seeded()).await;
    let points = format!(
        r#"{AGENT_POINTS}
  [[device.point]]
  id       = "setpoint"
  datatype = "int32"
  access   = "read_write"
  address  = {{ oid = "1.3.6.1.4.1.99999.1.20.0" }}

  [[device.point]]
  id       = "label"
  datatype = "string"
  access   = "read_write"
  address  = {{ oid = "1.3.6.1.4.1.99999.1.21.0" }}

  [[device.point]]
  id       = "limit"
  datatype = "uint32"
  access   = "read_write"
  address  = {{ oid = "1.3.6.1.4.1.99999.1.22.0", type = "counter32" }}

  [[device.point]]
  id       = "locked"
  datatype = "int32"
  access   = "read_write"
  address  = {{ oid = "1.3.6.1.4.1.99999.1.1.0" }}
"#
    );
    let mut conn = factory();
    conn.configure(&config(&agent_config(agent.address.port(), &points))).unwrap();
    conn.connect().await.unwrap();

    let write = |point: &str, value: serde_json::Value| tedge_dot_sdk::CommandRequest {
        point: point.to_string(),
        value: Some(value),
        value_repr: None,
        raw: None,
    };
    let device = DeviceId::from("agent");
    conn.execute(&device, "write", &write("setpoint", serde_json::json!(55))).await.unwrap();
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.20.0"), Some(Owned::Int(55)));

    conn.execute(&device, "write", &write("label", serde_json::json!("written"))).await.unwrap();
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.21.0"), Some(Owned::Str(b"written".to_vec())));

    // `type` decides what goes on the wire, not the point's datatype. `counter32` (tag 0x41) is
    // the discriminating choice: `gauge32` would prove nothing here, since a `uint32` point
    // already defaults to `unsigned32`, which shares tag 0x42 and the same encoder arm.
    conn.execute(&device, "write", &write("limit", serde_json::json!(250))).await.unwrap();
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.22.0"), Some(Owned::Counter32(250)));

    let refused = conn.execute(&device, "write", &write("locked", serde_json::json!(1))).await.unwrap_err();
    assert!(refused.to_string().contains("notWritable"), "{refused}");
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.1.0"), Some(Owned::Int(-42)), "unchanged");

    // A value the SNMP type cannot carry never leaves the connector.
    let refused = conn.execute(&device, "write", &write("limit", serde_json::json!(-1))).await.unwrap_err();
    assert!(refused.to_string().contains("out of range"), "{refused}");
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.22.0"), Some(Owned::Counter32(250)));

    // A raw write carries the type's content octets.
    let raw = tedge_dot_sdk::CommandRequest {
        point: "label".into(),
        value: None,
        value_repr: None,
        raw: Some("de ad be ef".into()),
    };
    conn.execute(&device, "write", &raw).await.unwrap();
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.21.0"), Some(Owned::Str(vec![0xde, 0xad, 0xbe, 0xef])));

    // A notification point is not writable at all.
    assert!(conn.execute(&device, "write", &write("sys_descr", serde_json::json!(1))).await.is_err());
}

#[tokio::test]
async fn an_snmpv3_authpriv_device_discovers_polls_and_writes() {
    let mut agent = Agent::seeded();
    let engine = LocalEngine::new(&unhex("80001f8880e2e0000000000009"), 5)
        .unwrap()
        .with_engine_time(1_000);
    // An agent authenticates with the user's keys localized to its OWN engine (RFC 3414 §A.2).
    let keys = engine.localize(&v3_agent_security()).unwrap();
    agent.v3 = Some((engine, keys));
    let agent = spawn_agent(agent).await;

    let text = format!(
        r#"
[connector]
protocol = "snmp"

[connection]
listen = "127.0.0.1:{listen}"
request_timeout = "1s"

[[device]]
name             = "v3"
protocol_address = {{ host = "127.0.0.1", port = {port}, version = "v3", v3 = {{ user = "rw-priv", level = "authPriv", auth_protocol = "SHA", auth_password = "auth-password-1", priv_protocol = "AES", priv_password = "priv-password-1" }} }}
default_mode     = "typed"

  [[device.point]]
  id       = "sys_descr"
  datatype = "string"
  address  = {{ oid = "1.3.6.1.2.1.1.1.0" }}

  [[device.point]]
  id       = "label"
  datatype = "string"
  access   = "read_write"
  address  = {{ oid = "1.3.6.1.4.1.99999.1.21.0" }}
"#,
        listen = free_port(),
        port = agent.address.port(),
    );
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    let reports = conn.connect().await.unwrap();
    assert_eq!(reports[0].info.as_ref().unwrap()["version"], "v3");

    let samples = read(&mut conn, "v3", &["sys_descr"]).await;
    let s = sample(&samples, "sys_descr");
    assert_eq!(
        s.value,
        Some(Value::Text("tedge-dot test agent".into())),
        "bad sample: {:?}; the agent saw {:?}",
        s.error,
        agent.log()
    );

    conn.execute(
        &DeviceId::from("v3"),
        "write",
        &tedge_dot_sdk::CommandRequest {
            point: "label".into(),
            value: Some(serde_json::json!("over authPriv")),
            value_repr: None,
            raw: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(agent.value("1.3.6.1.4.1.99999.1.21.0"), Some(Owned::Str(b"over authPriv".to_vec())));
    let log = agent.log();
    assert!(log.iter().any(|l| l.contains("v3 GetRequest") || l.contains("v3 GetBulkRequest")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("v3 SetRequest")), "{log:?}");
}

#[tokio::test]
async fn a_wrong_v3_password_gives_bad_samples_and_never_a_good_one() {
    let mut agent = Agent::seeded();
    let engine = LocalEngine::new(&unhex("80001f8880e2e0000000000010"), 2).unwrap();
    let keys = engine.localize(&v3_agent_security()).unwrap();
    agent.v3 = Some((engine, keys));
    let agent = spawn_agent(agent).await;
    let text = format!(
        r#"
[connector]
protocol = "snmp"
[connection]
listen = "127.0.0.1:{listen}"
request_timeout = "500ms"
retries = 0
[[device]]
name             = "v3"
protocol_address = {{ host = "127.0.0.1", port = {port}, version = "v3", v3 = {{ user = "rw-priv", level = "authPriv", auth_protocol = "SHA", auth_password = "wrong-password-9", priv_protocol = "AES", priv_password = "priv-password-1" }} }}
default_mode     = "typed"
  [[device.point]]
  id       = "sys_descr"
  datatype = "string"
  address  = {{ oid = "1.3.6.1.2.1.1.1.0" }}
"#,
        listen = free_port(),
        port = agent.address.port(),
    );
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();
    let samples = read(&mut conn, "v3", &["sys_descr"]).await;
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].quality, Quality::Bad);
    let error = samples[0].error.clone().unwrap_or_default();
    assert!(!error.contains("wrong-password-9") && !error.contains("priv-password-1"), "{error}");
    assert!(agent.log().iter().any(|l| l.contains("refused")), "{:?}", agent.log());
}

// ─── notifications ───────────────────────────────────────────────────────────────────────────

fn switch_config(port: u16, listen_host: &str) -> String {
    format!(
        r#"
[connector]
protocol = "snmp"

[connection]
listen    = "{listen_host}:{port}"
community = "public"
engine_id = "{RECEIVER_ENGINE_ID}"

[[device]]
name             = "switch"
protocol_address = {{ host = "127.0.0.1", v3 = {{ user = "trapuser", auth_protocol = "SHA", auth_password = "authpassword", priv_protocol = "AES", priv_password = "privpassword", engine_id = "{TRAP_ENGINE_ID}" }} }}

  [[device.point]]
  id       = "link_down"
  datatype = "string"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3" }}

  [[device.point]]
  id        = "if_index"
  datatype  = "int32"
  transform = {{ multiplier = 10 }}
  address   = {{ trap = "1.3.6.1.6.3.1.1.5.3", oid = "1.3.6.1.2.1.2.2.1.1" }}

  [[device.point]]
  id       = "if_name"
  datatype = "string"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3", oid = "1.3.6.1.2.1.31.1.1.1.1" }}

  [[device.point]]
  id       = "inform_text"
  mode     = "raw"
  address  = {{ trap = "1.3.6.1.4.1.99999.0.2", oid = "1.3.6.1.4.1.99999.2.1" }}
"#
    )
}

#[tokio::test]
async fn a_trap_becomes_samples_for_every_matching_point() {
    let port = free_port();
    let mut conn = factory();
    conn.configure(&config(&switch_config(port, "127.0.0.1"))).unwrap();
    let reports = conn.connect().await.unwrap();
    assert_eq!(reports[0].status, LinkStatus::Connected, "{:?}", reports[0].reason);
    let mut rx = subscribe(&mut conn, "switch", &["link_down", "if_index", "if_name", "inform_text"]).await;

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender.send_to(&unhex(LINK_DOWN), ("127.0.0.1", port)).await.unwrap();

    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.quality), ("link_down", Quality::Good));
    assert_eq!(s.value, Some(Value::Text("1.3.6.1.6.3.1.1.5.3".into())));
    assert_eq!(s.addr["source"], "127.0.0.1");
    assert_eq!(s.addr["version"], "v2c");
    assert_eq!(s.addr["pdu"], "trap");

    let s = next(&mut rx).await;
    assert_eq!(s.point, "if_index");
    assert_eq!(s.value, Some(Value::Number(30.0)), "ifIndex.3 = 3, scaled by the transform");
    assert_eq!(s.addr["oid"], "1.3.6.1.2.1.2.2.1.1.3");
    assert_eq!(s.raw, vec![3]);

    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.quality), ("if_name", Quality::Bad), "declared varbind missing");
    assert!(s.error.unwrap().contains("1.3.6.1.2.1.31.1.1.1.1"));

    nothing(&mut rx).await;
    conn.check_subscription(&DeviceId::from("switch")).await.unwrap();
    conn.disconnect().await.unwrap();
    assert!(conn.check_subscription(&DeviceId::from("switch")).await.is_err());
}

#[tokio::test]
async fn an_inform_is_acknowledged_and_a_rejected_community_is_not() {
    let port = free_port();
    let mut conn = factory();
    conn.configure(&config(&switch_config(port, "127.0.0.1"))).unwrap();
    conn.connect().await.unwrap();
    let mut rx = subscribe(&mut conn, "switch", &["inform_text", "link_down"]).await;

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let inform = unhex(INFORM);
    sender.send_to(&inform, ("127.0.0.1", port)).await.unwrap();

    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.mode, s.quality), ("inform_text", Mode::Raw, Quality::Good));
    assert_eq!(s.raw, b"inform me".to_vec());
    assert_eq!(s.addr["pdu"], "inform");

    let mut buf = [0u8; 1500];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), sender.recv_from(&mut buf))
        .await
        .expect("an acknowledgement within 3s")
        .unwrap();
    let response = Pdu::from_bytes(&buf[..len]).expect("the Response decodes");
    let request = Pdu::from_bytes(&inform).unwrap();
    assert_eq!(response.message_type, MessageType::Response);
    assert_eq!(response.req_id, request.req_id);
    assert_eq!((response.error_status, response.error_index), (0, 0));
    assert_eq!(response.varbinds.clone().count(), request.varbinds.clone().count());

    // A community the device does not accept: no sample, and no acknowledgement either.
    sender.send_to(&unhex(PRIVATE_TRAP), ("127.0.0.1", port)).await.unwrap();
    nothing(&mut rx).await;
    // ...and the receiver still works afterwards.
    sender.send_to(&unhex(LINK_DOWN), ("127.0.0.1", port)).await.unwrap();
    assert_eq!(next(&mut rx).await.point, "link_down");
}

#[tokio::test]
async fn a_notification_from_an_unconfigured_source_is_ignored_and_not_acknowledged() {
    if !ipv6_loopback() {
        eprintln!("skipped: no IPv6 loopback");
        return;
    }
    let port = free_port();
    let text = switch_config(port, "[::]");
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();
    let mut rx = subscribe(&mut conn, "switch", &["inform_text", "link_down"]).await;

    let stranger = UdpSocket::bind("[::1]:0").await.unwrap();
    stranger.send_to(&unhex(INFORM), ("::1", port)).await.unwrap();
    nothing(&mut rx).await;
    let mut buf = [0u8; 1500];
    assert!(
        tokio::time::timeout(Duration::from_millis(300), stranger.recv_from(&mut buf)).await.is_err(),
        "an inform from an unconfigured source must not be acknowledged"
    );
}

#[tokio::test]
async fn a_forwarded_notification_is_routed_by_snmp_trap_address() {
    if !ipv6_loopback() {
        eprintln!("skipped: no IPv6 loopback");
        return;
    }
    let port = free_port();
    let text = format!(
        r#"
[connector]
protocol = "snmp"

[connection]
listen     = "[::]:{port}"
community  = "public"
forwarders = ["::1"]

[[device]]
name             = "branch"
protocol_address = {{ host = "127.0.0.1" }}

  [[device.point]]
  id       = "link_down"
  datatype = "string"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3" }}

  [[device.point]]
  id       = "if_index"
  datatype = "int32"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3", oid = "1.3.6.1.2.1.2.2.1.1" }}
"#
    );
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();
    let mut rx = subscribe(&mut conn, "branch", &["link_down", "if_index"]).await;

    // A forwarder relays the original datagram and appends snmpTrapAddress.0 = the sender.
    let trap_oid = Oid::parse("1.3.6.1.6.3.1.1.4.1.0").unwrap().to_library();
    let if_index = Oid::parse("1.3.6.1.2.1.2.2.1.1.31").unwrap().to_library();
    let trap_address = Oid::parse("1.3.6.1.6.3.18.1.3.0").unwrap().to_library();
    let link_down = Oid::parse("1.3.6.1.6.3.1.1.5.3").unwrap().to_library();
    let forwarded = pdu::encode(
        LibVersion::V2C,
        b"public",
        MessageType::Trap,
        7,
        0,
        0,
        &[
            (&trap_oid, SnmpValue::ObjectIdentifier(link_down.clone())),
            (&if_index, SnmpValue::Integer(31)),
            (&trap_address, SnmpValue::IpAddress([127, 0, 0, 1])),
        ],
    )
    .unwrap();
    let forwarder = UdpSocket::bind("[::1]:0").await.unwrap();
    forwarder.send_to(&forwarded, ("::1", port)).await.unwrap();

    let s = next(&mut rx).await;
    assert_eq!(s.point, "link_down");
    assert_eq!(s.device, "branch");
    assert_eq!(s.addr["source"], "127.0.0.1", "the original sender, not the forwarder");
    assert_eq!(s.addr["forwarder"], "::1");
    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.value), ("if_index", Some(Value::Number(31.0))));

    // The same notification without snmpTrapAddress.0: nothing says whose it is.
    let plain = pdu::encode(
        LibVersion::V2C,
        b"public",
        MessageType::Trap,
        8,
        0,
        0,
        &[
            (&trap_oid, SnmpValue::ObjectIdentifier(link_down)),
            (&if_index, SnmpValue::Integer(32)),
        ],
    )
    .unwrap();
    forwarder.send_to(&plain, ("::1", port)).await.unwrap();
    nothing(&mut rx).await;
}

#[tokio::test]
async fn a_v3_trap_is_accepted_and_one_with_a_wrong_key_is_dropped() {
    let port = free_port();
    let mut conn = factory();
    conn.configure(&config(&switch_config(port, "127.0.0.1"))).unwrap();
    conn.connect().await.unwrap();
    let mut rx = subscribe(&mut conn, "switch", &["link_down", "if_index", "if_name"]).await;

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender.send_to(&unhex(V3_TRAP_WRONG_KEY), ("127.0.0.1", port)).await.unwrap();
    nothing(&mut rx).await;

    sender.send_to(&unhex(V3_TRAP), ("127.0.0.1", port)).await.unwrap();
    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.quality), ("link_down", Quality::Good));
    assert_eq!(s.addr["version"], "v3");
    assert_eq!(s.addr["pdu"], "trap");
    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.value), ("if_index", Some(Value::Number(30.0))));
}

#[tokio::test]
async fn a_v3_inform_is_discovered_acknowledged_and_delivered() {
    let port = free_port();
    let mut conn = factory();
    conn.configure(&config(&switch_config(port, "127.0.0.1"))).unwrap();
    conn.connect().await.unwrap();
    let mut rx = subscribe(&mut conn, "switch", &["inform_text"]).await;

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    async fn receive(socket: &UdpSocket) -> Vec<u8> {
        let mut buf = vec![0u8; 65535];
        let (len, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf))
            .await
            .expect("an answer within 3s")
            .unwrap();
        buf.truncate(len);
        buf
    }

    // 1. Discovery: an unauthenticated probe, answered with a Report carrying the engine.
    let anonymous = Security::new(b"", b"").with_auth(Auth::NoAuthNoPriv);
    let probe = v3::encode(
        &anonymous,
        &Outgoing {
            msg_id: 1,
            level: SecurityLevel::NoAuthNoPriv,
            reportable: true,
            engine_id: &[],
            engine_boots: 0,
            engine_time: 0,
            context_engine_id: &[],
            context_name: b"",
        },
        MessageType::GetRequest,
        4241,
        0,
        0,
        &[],
    )
    .unwrap();
    sender.send_to(&probe, ("127.0.0.1", port)).await.unwrap();
    let report = receive(&sender).await;
    let header = Header::parse(&report).expect("a Report");
    assert_eq!(header.engine_id, unhex(RECEIVER_ENGINE_ID), "the receiver's own engine");
    let (engine_id, boots, time) = (header.engine_id.to_vec(), header.engine_boots, header.engine_time);

    // 2. The inform itself, authenticated and encrypted for that engine.
    let keys = Security::new(b"trapuser", b"authpassword")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthPriv { cipher: Cipher::Aes128, privacy_password: b"privpassword".to_vec() })
        .with_engine_id(&engine_id)
        .unwrap();
    let trap_oid = Oid::parse("1.3.6.1.6.3.1.1.4.1.0").unwrap().to_library();
    let notification = Oid::parse("1.3.6.1.4.1.99999.0.2").unwrap().to_library();
    let payload = Oid::parse("1.3.6.1.4.1.99999.2.1").unwrap().to_library();
    let inform = v3::encode(
        &keys,
        &Outgoing {
            msg_id: 2,
            level: SecurityLevel::AuthPriv,
            reportable: true,
            engine_id: &engine_id,
            engine_boots: boots,
            engine_time: time,
            context_engine_id: &engine_id,
            context_name: b"",
        },
        MessageType::InformRequest,
        4242,
        0,
        0,
        &[
            (&trap_oid, SnmpValue::ObjectIdentifier(notification)),
            (&payload, SnmpValue::OctetString(b"hello-v3")),
        ],
    )
    .unwrap();
    sender.send_to(&inform, ("127.0.0.1", port)).await.unwrap();

    let s = next(&mut rx).await;
    assert_eq!((s.point.as_str(), s.quality), ("inform_text", Quality::Good));
    assert_eq!(s.raw, b"hello-v3".to_vec());
    assert_eq!(s.addr["version"], "v3");
    assert_eq!(s.addr["pdu"], "inform");

    // 3. ...and the sender gets its Response, under the receiver's engine.
    let response = receive(&sender).await;
    let mut view = Security::new(b"trapuser", b"authpassword")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthPriv { cipher: Cipher::Aes128, privacy_password: b"privpassword".to_vec() })
        .with_engine_id(&engine_id)
        .unwrap();
    let message = v3::receive_non_authoritative(&response, &mut view).expect("the Response verifies");
    assert_eq!(message.pdu.message_type, MessageType::Response);
    assert_eq!(message.pdu.req_id, 4242);
    assert_eq!(message.header.msg_id, 2);
}

#[tokio::test]
async fn object_points_are_polled_and_notification_points_pushed() {
    let agent = spawn_agent(Agent::seeded()).await;
    let text = format!(
        r#"
[connector]
protocol = "snmp"
[connection]
listen = "127.0.0.1:{listen}"
[[device]]
name             = "mixed"
protocol_address = {{ host = "127.0.0.1", port = {port} }}
default_mode     = "typed"
  [[device.point]]
  id       = "sys_descr"
  datatype = "string"
  address  = {{ oid = "1.3.6.1.2.1.1.1.0" }}
  [[device.point]]
  id       = "link_down"
  datatype = "string"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3" }}
"#,
        listen = free_port(),
        port = agent.address.port(),
    );
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.connect().await.unwrap();

    let device = DeviceId::from("mixed");
    assert!(!conn.pushes_point(&device, &refs(&["sys_descr"])[0]), "an object is polled");
    assert!(conn.pushes_point(&device, &refs(&["link_down"])[0]), "a notification point is pushed");
    assert!(!conn.pushes_point(&device, &refs(&["nope"])[0]), "an unknown point is not pushed");

    // Asking for a notification point produces nothing, rather than a bad sample every cycle.
    let samples = read(&mut conn, "mixed", &["link_down", "sys_descr"]).await;
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].point, "sys_descr");

    // ...and an object point cannot be subscribed.
    let (tx, _rx) = mpsc::channel(4);
    let err = conn.subscribe(&device, &refs(&["sys_descr"]), tx).await.unwrap_err();
    assert!(err.to_string().contains("polled"), "{err}");
}

/// `tedge-dot read`/`write` run next to the service, which already holds the notification port:
/// with push disabled the CLI connects and polls without binding it.
#[tokio::test]
async fn with_push_disabled_the_listen_port_is_left_to_the_service() {
    let agent = spawn_agent(Agent::seeded()).await;
    let service = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let text = format!(
        r#"
[connector]
protocol = "snmp"
[connection]
listen = "127.0.0.1:{listen}"
[[device]]
name             = "mixed"
protocol_address = {{ host = "127.0.0.1", port = {port} }}
default_mode     = "typed"
  [[device.point]]
  id       = "sys_descr"
  datatype = "string"
  address  = {{ oid = "1.3.6.1.2.1.1.1.0" }}
  [[device.point]]
  id       = "link_down"
  datatype = "string"
  address  = {{ trap = "1.3.6.1.6.3.1.1.5.3" }}
"#,
        listen = service.local_addr().unwrap().port(),
        port = agent.address.port(),
    );

    // The service's view: the port is taken, so the device cannot come up.
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    let reports = conn.connect().await.unwrap();
    assert_eq!(reports[0].status, LinkStatus::Disconnected);
    assert!(reports[0].reason.as_deref().unwrap_or_default().contains("cannot listen"), "{reports:?}");

    // The CLI's view: no socket is bound, and the object points are still read.
    let mut conn = factory();
    conn.configure(&config(&text)).unwrap();
    conn.disable_push();
    let reports = conn.connect().await.unwrap();
    assert_eq!(reports[0].status, LinkStatus::Connected, "{reports:?}");
    let samples = read(&mut conn, "mixed", &["sys_descr"]).await;
    assert_eq!(samples[0].quality, Quality::Good);
    let device = DeviceId::from("mixed");
    let reconnected = conn.reconnect(&device).await.unwrap();
    assert_eq!(reconnected.status, LinkStatus::Connected, "a reconnect does not bind it either");
    conn.disconnect().await.unwrap();
}

#[test]
fn invalid_configurations_are_rejected_naming_the_problem() {
    let base = |connection: &str, address: &str, point: &str| {
        format!(
            r#"
[connector]
protocol = "snmp"
[connection]
{connection}
[[device]]
name = "d"
protocol_address = {address}
  [[device.point]]
  id = "p"
  {point}
"#
        )
    };
    let ok_addr = r#"{ host = "10.0.0.1" }"#;
    let ok_point = r#"datatype = "int32"
  address = { oid = "1.3.6.1.2.1.2.2.1.1.0" }"#;
    let cases: Vec<(String, &str)> = vec![
        (base(r#"listen = "localhost:162""#, ok_addr, ok_point), "listen"),
        (base(r#"port = 162"#, ok_addr, ok_point), "port"),
        (base(r#"community = []"#, ok_addr, ok_point), "community"),
        (base(r#"engine_id = "0102""#, ok_addr, ok_point), "engine_id"),
        (base(r#"request_timeout = "soon""#, ok_addr, ok_point), "request_timeout"),
        (base("", r#"{ hostname = "10.0.0.1" }"#, ok_point), "hostname"),
        (base("", r#"{ host = "" }"#, ok_point), "host"),
        (base("", r#"{ host = "10.0.0.1", version = "v4" }"#, ok_point), "version"),
        (base("", r#"{ host = "10.0.0.1", version = "v3" }"#, ok_point), "v3"),
        (
            base("", r#"{ host = "10.0.0.1", version = "v3", v3 = { user = "u", auth_password = "short" } }"#, ok_point),
            "shorter than",
        ),
        (
            base("", r#"{ host = "10.0.0.1", version = "v3", v3 = { user = "u", auth_password = "long-enough-1", auth_password_file = "/x" } }"#, ok_point),
            "use one",
        ),
        (
            base("", r#"{ host = "10.0.0.1", version = "v3", v3 = { user = "u", auth_password = "long-enough-1" } }"#, ok_point),
            "auth_protocol",
        ),
        (
            base("", r#"{ host = "10.0.0.1", version = "v3", v3 = { user = "u", level = "authPriv", auth_protocol = "SHA", auth_password = "long-enough-1" } }"#, ok_point),
            "priv_protocol",
        ),
        (base("", ok_addr, r#"datatype = "int32"
  address = { varbind = "1.3.6" }"#), "varbind"),
        (base("", ok_addr, r#"datatype = "int32"
  address = { }"#), "address"),
        (base("", ok_addr, r#"datatype = "int32"
  address = { oid = "1.3.x" }"#), "address.oid"),
        (base("", ok_addr, r#"datatype = "string"
  address = { trap = [] }"#), "address.trap"),
        (base("", ok_addr, r#"datatype = "int32"
  access = "read_write"
  address = { trap = "1.3.6" }"#), "read-only"),
        (base("", ok_addr, r#"datatype = "int32"
  subscribe = false
  address = { trap = "1.3.6" }"#), "subscribe"),
        (base("", ok_addr, r#"datatype = "bytes"
  address = { oid = "1.3.6.1.0" }"#), "bytes"),
        (base("", ok_addr, r#"datatype = "int32"
  address = { trap = "1.3.6" }"#), "trap point"),
        (base("", ok_addr, r#"address = { oid = "1.3.6.1.0" }"#), "datatype"),
        // A writable float has no SNMP type to write unless the address names one.
        (base("", ok_addr, r#"datatype = "float64"
  access = "read_write"
  address = { oid = "1.3.6.1.0" }"#), "address.type"),
        (base("", ok_addr, r#"datatype = "int32"
  address = { trap = "1.3.6", oid = "1.3.6.1", type = "integer" }"#), "object points only"),
    ];
    for (text, needle) in cases {
        let err = factory().configure(&config(&text)).err().unwrap_or_else(|| panic!("accepted:\n{text}"));
        assert!(err.to_string().contains(needle), "error '{err}' should mention '{needle}'\n{text}");
    }

    let duplicate = r#"
[connector]
protocol = "snmp"
[[device]]
name = "a"
protocol_address = { host = "10.0.0.1" }
[[device]]
name = "b"
protocol_address = { host = "10.0.0.1" }
"#;
    let err = factory().configure(&config(duplicate)).unwrap_err();
    assert!(err.to_string().contains("one device only"), "{err}");

    let fine = base("", ok_addr, r#"mode = "raw"
  address = { trap = ["1.3.6.1.6.3.1.1.5.3", ".1.3.6.1.6.3.1.1.5.4"] }"#);
    factory().configure(&config(&fine)).unwrap();
}
