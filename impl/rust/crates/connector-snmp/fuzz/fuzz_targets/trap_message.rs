//! Fuzz the notification path: a receiver decodes whatever arrives on udp/162.
//!
//!   cd impl/rust/crates/connector-snmp && cargo +nightly fuzz run trap_message
//!
//! No input may panic — not the v1/v2c decode path, not SNMPv3 with either role of the
//! receiver (a trap under the sender's engine, an inform under ours), and not the conversions
//! or the acknowledgement built from whatever decoded.
#![no_main]

use connector_snmp::notification;
use connector_snmp::snmp2::v3::{self, Auth, AuthProtocol, Cipher, Header, LocalEngine, Security};
use connector_snmp::snmp2::Pdu;
use libfuzzer_sys::fuzz_target;
use tedge_dot_sdk::DataType;

const DATATYPES: &[DataType] = &[
    DataType::Bool,
    DataType::Int8,
    DataType::Uint8,
    DataType::Int16,
    DataType::Uint16,
    DataType::Int32,
    DataType::Uint32,
    DataType::Int64,
    DataType::Uint64,
    DataType::Float32,
    DataType::Float64,
    DataType::String,
];

fn user() -> Security {
    Security::new(b"trapuser", b"authpassword")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthPriv {
            cipher: Cipher::Aes128,
            privacy_password: b"privpassword".to_vec(),
        })
}

fuzz_target!(|data: &[u8]| {
    if Header::parse(data).is_ok() {
        // SNMPv3: the sender's engine (a trap), then ours (an inform and its discovery).
        let mut security = user();
        if let Ok(message) = v3::receive_non_authoritative(data, &mut security) {
            let _ = notification::from_pdu(&message.pdu, notification::Version::V3);
        }
        let mut engine = LocalEngine::new(b"\x80\x00\x00\x00\x05fuzz", 3).unwrap();
        let mut inform_keys = engine.localize(&user()).unwrap();
        match engine.receive(data, Some(&mut inform_keys)) {
            Ok(message) => {
                let _ = notification::from_pdu(&message.pdu, notification::Version::V3);
                let _ = engine.acknowledge(&message);
            }
            Err(refusal) => {
                // A Report must itself be a message this receiver can read back.
                if let Some(report) = refusal.report {
                    assert!(Header::parse(&report).is_ok(), "a Report that does not parse");
                }
            }
        }
        return;
    }

    let Ok(n) = notification::decode(data) else { return };
    let _ = n.trap_address();
    assert!(!n.trap.encode().is_empty());
    if n.pdu == notification::PduKind::Inform {
        let pdu = Pdu::from_bytes(data).expect("it decoded a moment ago");
        let _ = notification::inform_response(&pdu);
    }
    for vb in &n.varbinds {
        let _ = vb.value.text();
        for &dt in DATATYPES {
            let _ = vb.value.convert(dt);
        }
    }
});
