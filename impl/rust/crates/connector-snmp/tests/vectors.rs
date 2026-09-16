//! The shared golden vectors (`connectors/snmp/conformance/trap-vectors.json`, spec §8): the C
//! build's `tests/snmp.c` reads the same file, and both are checked against a reference decoder
//! neither implementation produced.
//!
//! Everything goes through the library-based decode path the connector itself uses.

use connector_snmp::notification::{self, Version};
use connector_snmp::snmp2::v3::{
    self, Auth, AuthProtocol, Cipher, Header, KeyExtension, LocalEngine, Security,
};
use connector_snmp::snmp2::{MessageType, Value as SnmpValue};
use connector_snmp::value::{self, VarValue};
use serde_json::Value as Json;
use tedge_dot_sdk::{DataType, Value};

const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../connectors/snmp/conformance/trap-vectors.json"
));

fn vectors() -> Json {
    serde_json::from_str(VECTORS).expect("trap-vectors.json parses")
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

/// Decode one varbind value from a vector's type name and content octets, through the library.
fn decode_value(kind: &str, content: &[u8]) -> VarValue {
    let tag = tag_of(kind);
    let mut element = vec![tag];
    if content.len() < 0x80 {
        element.push(content.len() as u8);
    } else {
        element.push(0x82);
        element.extend((content.len() as u16).to_be_bytes());
    }
    element.extend_from_slice(content);
    let value = connector_snmp::snmp2::AsnReader::from_bytes(&element)
        .read_value()
        .unwrap_or_else(|e| panic!("{kind} {} does not decode: {e}", value::hex(content)));
    VarValue::from_library(&value).unwrap_or_else(|e| panic!("{kind}: {e}"))
}

/// The tag a vector's type name stands for (`unknown` uses an unassigned application tag).
fn tag_of(name: &str) -> u8 {
    match name {
        "integer" => 0x02,
        "octet_string" => 0x04,
        "null" => 0x05,
        "oid" => 0x06,
        "ip_address" => 0x40,
        "counter32" => 0x41,
        "gauge32" => 0x42,
        "timeticks" => 0x43,
        "opaque" => 0x44,
        "counter64" => 0x46,
        "no_such_object" => 0x80,
        "no_such_instance" => 0x81,
        "end_of_mib_view" => 0x82,
        "unknown" => 0x47,
        other => panic!("unknown type name in vectors: {other}"),
    }
}

/// Check one decoded notification against a vector's expectation.
fn check(name: &str, n: &notification::Notification, expect: &Json) {
    assert_eq!(n.version.as_str(), expect["version"], "{name}: version");
    assert_eq!(value::hex(&n.community), expect["community"], "{name}: community");
    assert_eq!(n.pdu.as_str(), expect["pdu"], "{name}: pdu");
    assert_eq!(
        n.request_id.map(|v| v.to_string()).as_deref(),
        expect["request_id"].as_str(),
        "{name}: request_id"
    );
    assert_eq!(
        n.agent_addr.as_ref().map(value::dotted_ip).as_deref(),
        expect["agent_addr"].as_str(),
        "{name}: agent_addr"
    );
    assert_eq!(
        n.uptime.map(|v| v.to_string()).as_deref(),
        expect["uptime"].as_str(),
        "{name}: uptime"
    );
    assert_eq!(n.trap.to_string(), expect["trap"], "{name}: trap");
    assert_eq!(value::hex(&n.trap.encode()), expect["trap_raw"], "{name}: trap_raw");

    let want = expect["varbinds"].as_array().unwrap();
    assert_eq!(n.varbinds.len(), want.len(), "{name}: varbind count");
    for (i, (got, want)) in n.varbinds.iter().zip(want).enumerate() {
        assert_eq!(got.name.to_string(), want["oid"], "{name}: varbind {i} oid");
        assert_eq!(got.value.kind.name(), want["type"], "{name}: varbind {i} type");
        assert_eq!(got.value.text().as_deref(), want["value"].as_str(), "{name}: varbind {i} value");
        assert_eq!(value::hex(&got.value.raw), want["raw"], "{name}: varbind {i} raw");
    }
}

#[test]
fn every_message_decodes_as_expected() {
    let doc = vectors();
    let messages = doc["messages"].as_array().unwrap();
    assert!(messages.len() >= 20, "the vector file lost its messages");
    for m in messages {
        let name = m["name"].as_str().unwrap();
        let n = notification::decode(&unhex(m["hex"].as_str().unwrap()))
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        check(name, &n, &m["expect"]);
    }
}

#[test]
fn every_malformed_datagram_is_rejected() {
    let doc = vectors();
    let malformed = doc["malformed"].as_array().unwrap();
    assert!(malformed.len() >= 40, "the vector file lost its malformed datagrams");
    for m in malformed {
        let name = m["name"].as_str().unwrap();
        // Nothing from the decoded notification: it carries the v3 security state, and
        // printing any part of it would risk key material in the test log. The vector's
        // own name and reason identify the failure.
        if notification::decode(&unhex(m["hex"].as_str().unwrap())).is_ok() {
            panic!("{name}: decoded, but must be rejected ({})", m["why"]);
        }
    }
}

#[test]
fn every_conversion_matches() {
    let doc = vectors();
    for c in doc["conversions"].as_array().unwrap() {
        let label = format!("{} {} -> {}", c["type"], c["raw"], c["datatype"]);
        let value = decode_value(c["type"].as_str().unwrap(), &unhex(c["raw"].as_str().unwrap()));
        let datatype: DataType = serde_json::from_value(c["datatype"].clone()).unwrap();
        let got = value.convert(datatype);
        if c["bad"] == true {
            assert!(got.is_err(), "{label}: must be bad, got {got:?}");
            continue;
        }
        let got = got.unwrap_or_else(|e| panic!("{label}: must convert, got bad: {e}"));
        assert_eq!(got.repr(), c["repr"], "{label}: value_repr");
        match got {
            Value::Number(n) => assert_eq!(n, c["value"].as_f64().unwrap(), "{label}"),
            Value::Bool(b) => assert_eq!(b, c["value"].as_bool().unwrap(), "{label}"),
            Value::Text(t) => assert_eq!(t, c["value"].as_str().unwrap(), "{label}"),
        }
    }
}

// ─── SNMPv3 ──────────────────────────────────────────────────────────────────────────────────

/// The credentials a `v3.users` entry describes.
fn user(users: &Json, id: &str) -> Security {
    let u = &users[id];
    let name = u["user"].as_str().unwrap().as_bytes();
    let auth_password = u["auth_password"].as_str().unwrap_or_default().as_bytes();
    let protocol = match u["auth_protocol"].as_str() {
        Some("MD5") => AuthProtocol::Md5,
        Some("SHA") | Some("SHA1") => AuthProtocol::Sha1,
        Some("SHA256") => AuthProtocol::Sha256,
        Some(other) => panic!("unsupported auth protocol in the vectors: {other}"),
        None => AuthProtocol::Sha1,
    };
    let cipher = match u["priv_protocol"].as_str() {
        Some("DES") => Some(Cipher::Des),
        Some("AES") | Some("AES128") => Some(Cipher::Aes128),
        Some("AES256") => Some(Cipher::Aes256),
        Some(other) => panic!("unsupported privacy protocol in the vectors: {other}"),
        None => None,
    };
    let auth = match (u["auth_protocol"].as_str(), cipher) {
        (None, _) => Auth::NoAuthNoPriv,
        (Some(_), None) => Auth::AuthNoPriv,
        (Some(_), Some(cipher)) => Auth::AuthPriv {
            cipher,
            privacy_password: u["priv_password"].as_str().unwrap_or_default().as_bytes().to_vec(),
        },
    };
    let mut security = Security::new(name, auth_password)
        .with_auth_protocol(protocol)
        .with_auth(auth);
    if matches!(cipher, Some(Cipher::Aes192 | Cipher::Aes256)) {
        security = security.with_key_extension_method(KeyExtension::Blumenthal);
    }
    security
}

#[test]
fn every_v3_message_decodes_as_expected() {
    let doc = vectors();
    let v3_doc = &doc["v3"];
    let messages = v3_doc["messages"].as_array().expect("the vector file has a v3 section");
    assert!(!messages.is_empty(), "the v3 section lost its messages");
    for m in messages {
        let name = m["name"].as_str().unwrap();
        let bytes = unhex(m["hex"].as_str().unwrap());
        let mut security = user(&v3_doc["users"], m["user"].as_str().unwrap());
        let message = v3::receive_non_authoritative(&bytes, &mut security)
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        let expect = &m["expect"];
        assert_eq!(
            value::hex(message.header.engine_id),
            expect["engine_id"],
            "{name}: engine ID"
        );
        assert_eq!(
            format!("{:?}", message.header.security_level()).to_lowercase(),
            expect["level"].as_str().unwrap().to_lowercase(),
            "{name}: security level"
        );
        assert_eq!(
            value::hex(message.context_engine_id),
            expect["context_engine_id"],
            "{name}: context engine ID"
        );
        assert_eq!(
            String::from_utf8_lossy(message.context_name),
            expect["context_name"].as_str().unwrap(),
            "{name}: context name"
        );
        let n = notification::from_pdu(&message.pdu, Version::V3)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(name, &n, expect);
        // ...and the engine ID was learned from a message that passed.
        drop(message);
        assert_eq!(value::hex(security.engine_id()), expect["engine_id"], "{name}: learned engine");
    }
}

#[test]
fn every_rejected_v3_message_is_rejected() {
    let doc = vectors();
    let v3_doc = &doc["v3"];
    let rejected = v3_doc["rejected"].as_array().expect("the v3 section has rejections");
    assert!(!rejected.is_empty());
    for m in rejected {
        let name = m["name"].as_str().unwrap();
        let bytes = unhex(m["hex"].as_str().unwrap());
        let mut security = user(&v3_doc["users"], m["user"].as_str().unwrap());
        let outcome = v3::receive_non_authoritative(&bytes, &mut security);
        assert!(outcome.is_err(), "{name}: accepted, but must be rejected ({})", m["why"]);
        assert!(
            security.engine_id().is_empty(),
            "{name}: a message that fails its checks must teach us nothing"
        );
    }
}

#[test]
fn a_discovery_probe_is_answered_with_this_engines_identity() {
    let doc = vectors();
    let v3_doc = &doc["v3"];
    let receiver = &v3_doc["receiver"];
    let engine_id = unhex(receiver["engine_id"].as_str().unwrap());
    let boots = receiver["engine_boots"].as_i64().unwrap();
    let time = receiver["engine_time"].as_i64().unwrap();

    for probe in v3_doc["probes"].as_array().expect("the v3 section has probes") {
        let name = probe["name"].as_str().unwrap();
        let bytes = unhex(probe["hex"].as_str().unwrap());
        let mut engine = LocalEngine::new(&engine_id, boots).unwrap().with_engine_time(time);
        let refusal = engine
            .receive(&bytes, None)
            .err()
            .unwrap_or_else(|| panic!("{name}: a probe carries no engine ID, so it must be refused"));
        let report = refusal.report.unwrap_or_else(|| panic!("{name}: a reportable message"));
        let expect = &probe["expect"];

        let header = Header::parse(&report).unwrap_or_else(|e| panic!("{name}: report: {e}"));
        assert_eq!(header.msg_id.to_string(), expect["msg_id"].as_str().unwrap(), "{name}: msgID");
        assert_eq!(value::hex(header.engine_id), value::hex(&engine_id), "{name}: engine ID");
        assert_eq!(header.engine_boots, boots, "{name}: engine boots");
        assert!((time..=time + 5).contains(&header.engine_time), "{name}: engine time");
        assert!(!header.is_reportable(), "{name}: a Report is never reportable");

        // The Report is unauthenticated, so anyone can read it — which is the point.
        let mut anyone = Security::new(header.user_name, b"").with_auth(Auth::NoAuthNoPriv);
        let message = v3::receive_non_authoritative(&report, &mut anyone)
            .unwrap_or_else(|e| panic!("{name}: the report does not decode: {e}"));
        assert_eq!(message.pdu.message_type, MessageType::Report, "{name}");
        assert_eq!(
            message.pdu.req_id.to_string(),
            expect["request_id"].as_str().unwrap(),
            "{name}: the probe's request-id"
        );
        let varbinds: Vec<(String, String)> = message
            .pdu
            .varbinds
            .clone()
            .map(|(oid, value)| {
                let text = match value {
                    SnmpValue::Counter32(v) => v.to_string(),
                    other => panic!("{name}: a usmStats counter, not {other:?}"),
                };
                (oid.to_id_string(), text)
            })
            .collect();
        assert_eq!(varbinds.len(), 1, "{name}");
        assert_eq!(varbinds[0].0, expect["counter"].as_str().unwrap(), "{name}: the usmStats OID");
        assert_eq!(varbinds[0].1, "1", "{name}: the counter's value");
    }
}
