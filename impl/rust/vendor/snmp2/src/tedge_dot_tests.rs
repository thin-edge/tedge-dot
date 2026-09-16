//! Tests of the tedge-dot patches (see TEDGE-DOT-PATCH.md), one section per patch.

use crate::{
    Error, MessageType, Oid, Pdu, Value, Varbinds, Version, pdu,
    v3::{
        self, Auth, AuthErrorKind, AuthProtocol, Cipher, Header, LocalEngine, Outgoing, Security,
        SecurityLevel,
    },
};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn oid(arcs: &[u64]) -> Oid<'static> {
    Oid::from(arcs).unwrap()
}

/// `tlv(tag, content)` with a short-form length.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    assert!(content.len() < 128);
    let mut out = vec![tag, content.len() as u8];
    out.extend_from_slice(content);
    out
}

/// A v2c SNMPv2-Trap message around an encoded varbind list.
fn v2c_trap(varbind_list: &[u8]) -> Vec<u8> {
    let mut pdu_body = tlv(0x02, &[7]);
    pdu_body.extend(tlv(0x02, &[0]));
    pdu_body.extend(tlv(0x02, &[0]));
    pdu_body.extend(tlv(0x30, varbind_list));
    let mut msg = tlv(0x02, &[1]);
    msg.extend(tlv(0x04, b"public"));
    msg.extend(tlv(0xA7, &pdu_body));
    tlv(0x30, &msg)
}

fn varbind(name: &[u8], value: &[u8]) -> Vec<u8> {
    let mut body = tlv(0x06, name);
    body.extend_from_slice(value);
    tlv(0x30, &body)
}

const TRAP_OID_NAME: &[u8] = &[0x2b, 6, 1, 6, 3, 1, 1, 4, 1, 0];

fn values_of(pdu: &Pdu<'_>) -> Vec<String> {
    pdu.varbinds
        .clone()
        .map(|(name, value)| format!("{name}={value:?}"))
        .collect()
}

// ─── 1: strict varbind lists ─────────────────────────────────────────────────────────────────

#[test]
fn a_truncated_varbind_list_rejects_the_message() {
    let good = [
        varbind(TRAP_OID_NAME, &tlv(0x06, &[0x2b, 6, 1])),
        varbind(&[0x2b, 6, 1, 2], &tlv(0x02, &[5])),
    ]
    .concat();
    let message = v2c_trap(&good);
    let pdu = Pdu::from_bytes(&message).unwrap();
    assert_eq!(pdu.varbinds.check(), Ok(2));

    // A second varbind without its value: upstream iterated one varbind and stopped.
    let truncated = [
        varbind(TRAP_OID_NAME, &tlv(0x06, &[0x2b, 6, 1])),
        tlv(0x30, &tlv(0x06, &[0x2b, 6, 1, 2])),
    ]
    .concat();
    assert!(Pdu::from_bytes(&v2c_trap(&truncated)).is_err());
    // An element that is not a SEQUENCE.
    let not_a_sequence = [
        varbind(TRAP_OID_NAME, &tlv(0x06, &[0x2b, 6, 1])),
        tlv(0x31, &tlv(0x06, &[0x2b])),
    ]
    .concat();
    assert!(Pdu::from_bytes(&v2c_trap(&not_a_sequence)).is_err());
    assert!(Varbinds::from_bytes(&not_a_sequence).check().is_err());
}

#[test]
fn unknown_tags_decode_and_multi_octet_tags_do_not() {
    let list = [
        varbind(&[0x2b, 1], &tlv(0x47, &[1, 2])),
        varbind(&[0x2b, 2], &tlv(0x01, &[1])),
        varbind(&[0x2b, 3], &tlv(0x83, &[])),
    ]
    .concat();
    let message = v2c_trap(&list);
    let pdu = Pdu::from_bytes(&message).unwrap();
    let values: Vec<Value> = pdu.varbinds.clone().map(|(_, v)| v).collect();
    assert!(matches!(values[0], Value::Unknown(0x47, [1, 2])));
    assert!(matches!(values[1], Value::Boolean(true)), "BOOLEAN decodes");
    assert!(matches!(values[2], Value::Unknown(0x83, [])));

    let multi_octet = varbind(&[0x2b, 1], &[0x5F, 0x81, 0x01, 0x00]);
    assert!(Pdu::from_bytes(&v2c_trap(&multi_octet)).is_err());
}

#[test]
fn received_unknown_and_constructed_values_are_echoed() {
    let list = [
        varbind(&[0x2b, 1], &tlv(0x47, &[1, 2])),
        varbind(&[0x2b, 2], &tlv(0x30, &tlv(0x02, &[9]))),
    ]
    .concat();
    let bytes = v2c_trap(&list);
    let mut pdu = Pdu::from_bytes(&bytes).unwrap();
    pdu.message_type = MessageType::Trap;
    assert_eq!(pdu.to_bytes().unwrap(), bytes);
}

// ─── 2: value ranges and OID validity ────────────────────────────────────────────────────────

fn value_of(value_tlv: &[u8]) -> Result<Vec<String>, Error> {
    let bytes = v2c_trap(&varbind(&[0x2b, 6, 1], value_tlv));
    Pdu::from_bytes(&bytes).map(|pdu| values_of(&pdu))
}

#[test]
fn unsigned_values_are_range_checked_not_narrowed() {
    assert_eq!(value_of(&tlv(0x41, &[0xff, 0xff, 0xff, 0xff])).unwrap(), ["1.3.6.1=COUNTER32: 4294967295"]);
    assert_eq!(value_of(&tlv(0x42, &[0, 0xff, 0xff, 0xff, 0xff])).unwrap(), ["1.3.6.1=UNSIGNED32: 4294967295"]);
    // 2^32: upstream read it as i64 and cast it to 0.
    assert!(value_of(&tlv(0x41, &[1, 0, 0, 0, 0])).is_err());
    assert!(value_of(&tlv(0x43, &[0, 0, 0, 0, 0, 1])).is_err(), "six octets");
    assert!(value_of(&tlv(0x43, &[])).is_err(), "no octets");
    // 2^64 - 1 needs nine octets: upstream refused it (and so ended the varbind list).
    let max = [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(value_of(&tlv(0x46, &max)).unwrap(), ["1.3.6.1=COUNTER64: 18446744073709551615"]);
    assert!(value_of(&tlv(0x46, &[1, 0, 0, 0, 0, 0, 0, 0, 0])).is_err());
    assert!(value_of(&tlv(0x02, &[])).is_err(), "an INTEGER without octets");
}

#[test]
fn malformed_oids_are_rejected() {
    assert!(value_of(&tlv(0x06, &[])).is_err(), "empty");
    assert!(value_of(&tlv(0x06, &[0x2b, 0x86])).is_err(), "dangling continuation");
    assert!(value_of(&tlv(0x06, &[0x2b, 0x90, 0x80, 0x80, 0x80, 0x00])).is_err(), "arc of 2^32");
    assert!(value_of(&tlv(0x06, &[0x2b, 0x8f, 0xff, 0xff, 0xff, 0x7f])).is_ok(), "arc of 2^32-1");
    assert!(value_of(&tlv(0x06, &[0x2b, 0x80, 0x01])).is_ok(), "non-minimal padding decodes");
    let bad_name = v2c_trap(&varbind(&[0x2b, 0x86], &tlv(0x05, &[])));
    assert!(Pdu::from_bytes(&bad_name).is_err());
}

#[test]
fn counter64_encodes_as_unsigned() {
    let name = oid(&[1, 3, 6, 1]);
    for (value, content) in [
        (u64::MAX, vec![0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
        (1 << 63, vec![0, 0x80, 0, 0, 0, 0, 0, 0, 0]),
        (0x80, vec![0, 0x80]),
        (0, vec![0]),
    ] {
        let bytes = pdu::encode(Version::V2C, b"public", MessageType::SetRequest, 1, 0, 0, &[(&name, Value::Counter64(value))]).unwrap();
        assert!(bytes.windows(content.len() + 2).any(|w| w[0] == 0x46 && w[1] as usize == content.len() && w[2..] == content[..]), "{value}");
        let pdu = Pdu::from_bytes(&bytes).unwrap();
        assert_eq!(values_of(&pdu), [format!("1.3.6.1=COUNTER64: {value}")]);
    }
}

#[test]
fn encode_builds_v1_and_v2c_messages() {
    let name = oid(&[1, 3, 6, 1, 2, 1, 1, 3, 0]);
    let bytes = pdu::encode(Version::V1, b"private", MessageType::Response, -9, 2, 1, &[(&name, Value::Timeticks(5))]).unwrap();
    let pdu = Pdu::from_bytes(&bytes).unwrap();
    assert_eq!((pdu.version().unwrap(), pdu.community, pdu.message_type), (Version::V1, &b"private"[..], MessageType::Response));
    assert_eq!((pdu.req_id, pdu.error_status, pdu.error_index), (-9, 2, 1));
    assert!(pdu::encode(Version::V3, b"", MessageType::Response, 1, 0, 0, &[]).is_err());
    assert!(pdu::encode(Version::V1, b"", MessageType::TrapV1, 1, 0, 0, &[]).is_err());
}

// ─── 3: no write through the shared input ────────────────────────────────────────────────────

fn agent_security(engine_id: &[u8]) -> Security {
    Security::new(b"pollster", b"authpassword1")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthPriv { cipher: Cipher::Aes128, privacy_password: b"privpassword1".to_vec() })
        .with_engine_id(engine_id)
        .unwrap()
}

#[test]
fn parsing_an_authenticated_message_leaves_the_input_untouched() {
    let engine_id = unhex("80001f8804746573742d61");
    let agent = agent_security(&engine_id);
    let sysdescr = oid(&[1, 3, 6, 1, 2, 1, 1, 1, 0]);
    let response = v3::encode(
        &agent,
        &Outgoing {
            msg_id: 77,
            level: SecurityLevel::AuthNoPriv,
            reportable: false,
            engine_id: &engine_id,
            engine_boots: 3,
            engine_time: 1000,
            context_engine_id: &engine_id,
            context_name: b"",
        },
        MessageType::Response,
        77,
        0,
        0,
        &[(&sysdescr, Value::OctetString(b"agent"))],
    )
    .unwrap();

    let mut client = Security::new(b"pollster", b"authpassword1")
        .with_auth_protocol(AuthProtocol::Sha1)
        .with_auth(Auth::AuthNoPriv)
        .with_engine_id(&engine_id)
        .unwrap()
        .with_engine_boots_and_time(3, 1000);
    let input = response.clone();
    let pdu = Pdu::from_bytes_with_security(&input, Some(&mut client)).unwrap();
    assert_eq!(pdu.req_id, 77);
    drop(pdu);
    // Upstream zeroed the authentication parameters inside `input` itself.
    assert_eq!(input, response);
}

// ─── 4: stale responses are skipped ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_late_response_to_an_earlier_request_is_skipped() {
    let agent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = agent.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            let (len, peer) = agent.recv_from(&mut buf).await.unwrap();
            let request = Pdu::from_bytes(&buf[..len]).unwrap();
            let name = request.varbinds.clone().next().unwrap().0;
            // First the answer to the previous request, as if it had been late; then this one's.
            for req_id in [request.req_id.wrapping_sub(1), request.req_id] {
                let answer = pdu::encode(Version::V2C, b"public", MessageType::Response, req_id, 0, 0, &[(&name, Value::Integer(i64::from(req_id)))]).unwrap();
                agent.send_to(&answer, peer).await.unwrap();
            }
        }
    });
    let mut session = crate::AsyncSession::new_v2c(addr, b"public", 100).await.unwrap();
    let name = oid(&[1, 3, 6, 1, 2, 1, 1, 3, 0]);
    for expected in [100, 101, 102] {
        let pdu = tokio::time::timeout(std::time::Duration::from_secs(2), session.get(&name))
            .await
            .expect("no hang")
            .expect("the stale response is skipped, not reported as a mismatch");
        assert_eq!(values_of(&pdu), [format!("1.3.6.1.2.1.1.3.0=INTEGER: {expected}")]);
    }
}

// ─── 5: receiver-side USM ────────────────────────────────────────────────────────────────────

const TRAP_ENGINE: &str = "8000000001020304";
/// net-snmp 5.6 `snmptrap -v 3 -e 0x8000000001020304 -u trapuser -l authPriv -a SHA -A authpassword -x AES -X privpassword ... 12345 linkDown ifIndex.3 i 3`
const TRAP_SHA_AES: &str = "3081af0201033011020470f05afb020300ffe3040103020103043430320408800000000102030402010002010004087472617075736572040c050ce04e23b31f144145ce6d040851e6aaec28b3400d046185e6a5a39a4498677e1d16ef49f4bed0af373de4554b49a387e22e269c881f7e11677c9eb80b812195e9f121437b4638c6bfd46c938388be9e0904d5cd97ce5657f7cffffb7f37abc697543443bb29c37323c37e5c06807d5ff38cd2b69101ca78";
/// The same with `-A wrongpassword`.
const TRAP_WRONG_PASSWORD: &str = "3081af02010330110204675caea0020300ffe3040103020103043430320408800000000102030402010002010004087472617075736572040c18986172bf59f9b8be0f5c1e0408ef501818bc82e9c20461e3266a52844b5a841c83c40fcde9e95e36f499443acf35a97228b1f0c232c94f500290556c77c2133775f1a522a361ce46f29968286f451bdddad61e2aecb69dd73c86a095a91050161e0cb51704a4c0ac5ff129222963404b2a3b7da112fb64c4";
/// The same with `-a MD5 -x DES`: net-snmp pads the DES plaintext with zeros.
const TRAP_MD5_DES: &str = "3081b60201033011020420ea51ff020300ffe3040103020103043430320408800000000102030402010002010004087472617075736572040cfb1e19b5f6d19d2604cc0ad0040800000001a651b89004686c2529d00201f842f98527b02104a9d710363df784aa7ed1e23879f047aaf342d8ff6cfad5b312cb1eee7a06381fa8b68200b323c52175e3a42b1d4d5f30b60d090addd4e3cf16f1c5be3f95af59336a034242a689b9a67c7ca4d2e90abfc442267121c883bf36b4";
/// net-snmp 5.6 `snmpinform -v 3 ...`: the engine ID discovery probe it sends first.
const INFORM_PROBE: &str = "304f0201033011020470094ab2020300ffe30401040201030410300e04000201000201000400040004003025041180001f888016cd170c1b86a96a000000000400a00e020446e2c5420201000201003000";

fn trap_user(cipher: Cipher, auth: AuthProtocol) -> Security {
    Security::new(b"trapuser", b"authpassword")
        .with_auth_protocol(auth)
        .with_auth(Auth::AuthPriv { cipher, privacy_password: b"privpassword".to_vec() })
}

const LINK_DOWN: [&str; 3] = [
    "1.3.6.1.2.1.1.3.0=TIMETICKS: 12345",
    "1.3.6.1.6.3.1.1.4.1.0=OBJECT IDENTIFIER: 1.3.6.1.6.3.1.1.5.3",
    "1.3.6.1.2.1.2.2.1.1.3=INTEGER: 3",
];

#[test]
fn net_snmp_v3_traps_decode_with_the_senders_engine() {
    for (hex, cipher, auth) in [
        (TRAP_SHA_AES, Cipher::Aes128, AuthProtocol::Sha1),
        (TRAP_MD5_DES, Cipher::Des, AuthProtocol::Md5),
    ] {
        let bytes = unhex(hex);
        let mut user = trap_user(cipher, auth);
        let msg = v3::receive_non_authoritative(&bytes, &mut user).unwrap();
        assert_eq!(msg.pdu.message_type, MessageType::Trap);
        assert_eq!(msg.header.security_level(), SecurityLevel::AuthPriv);
        assert!(!msg.header.is_reportable());
        assert_eq!(values_of(&msg.pdu), LINK_DOWN, "{cipher:?}");
        drop(msg);
        assert_eq!(user.engine_id(), unhex(TRAP_ENGINE), "learned from the first authenticated trap");
        // The same capture again: boots 0 and time 0 do not go backwards.
        assert!(v3::receive_non_authoritative(&bytes, &mut user).is_ok());
    }
}

#[test]
fn a_v3_trap_with_a_wrong_key_or_engine_changes_nothing() {
    let bytes = unhex(TRAP_WRONG_PASSWORD);
    let mut user = trap_user(Cipher::Aes128, AuthProtocol::Sha1);
    assert_eq!(
        v3::receive_non_authoritative(&bytes, &mut user).unwrap_err(),
        Error::AuthFailure(AuthErrorKind::SignatureMismatch)
    );
    assert!(user.engine_id().is_empty(), "an unauthenticated trap teaches no engine ID");

    let mut pinned = trap_user(Cipher::Aes128, AuthProtocol::Sha1).with_engine_id(&unhex("8000000001020305")).unwrap();
    assert_eq!(
        v3::receive_non_authoritative(&unhex(TRAP_SHA_AES), &mut pinned).unwrap_err(),
        Error::AuthFailure(AuthErrorKind::EngineIdMismatch)
    );

    let mut other_user = Security::new(b"someone", b"authpassword").with_auth_protocol(AuthProtocol::Sha1);
    assert_eq!(
        v3::receive_non_authoritative(&unhex(TRAP_SHA_AES), &mut other_user).unwrap_err(),
        Error::AuthFailure(AuthErrorKind::UsernameMismatch)
    );
    let mut no_auth = Security::new(b"trapuser", b"").with_auth(Auth::NoAuthNoPriv);
    assert_eq!(
        v3::receive_non_authoritative(&unhex(TRAP_SHA_AES), &mut no_auth).unwrap_err(),
        Error::AuthFailure(AuthErrorKind::UnsupportedSecLevel)
    );
}

fn trap_from(sender: &LocalEngine, user: &Security, boots: i64, time: i64) -> Vec<u8> {
    let name = oid(&[1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0]);
    let trap = oid(&[1, 3, 6, 1, 6, 3, 1, 1, 5, 1]);
    v3::encode(
        user,
        &Outgoing {
            msg_id: 5,
            level: SecurityLevel::AuthPriv,
            reportable: false,
            engine_id: sender.engine_id(),
            engine_boots: boots,
            engine_time: time,
            context_engine_id: sender.engine_id(),
            context_name: b"",
        },
        MessageType::Trap,
        9,
        0,
        0,
        &[(&name, Value::ObjectIdentifier(trap))],
    )
    .unwrap()
}

#[test]
fn a_replayed_v3_trap_is_outside_the_time_window() {
    let sender = LocalEngine::new(&unhex("8000000005aabbccdd"), 4).unwrap();
    let keys = sender.localize(&trap_user(Cipher::Aes128, AuthProtocol::Sha1)).unwrap();
    let mut receiver = trap_user(Cipher::Aes128, AuthProtocol::Sha1);
    assert!(v3::receive_non_authoritative(&trap_from(&sender, &keys, 4, 1000), &mut receiver).is_ok());
    assert!(v3::receive_non_authoritative(&trap_from(&sender, &keys, 4, 900), &mut receiver).is_ok(), "within 150 s");
    assert!(v3::receive_non_authoritative(&trap_from(&sender, &keys, 4, 849), &mut receiver).is_err(), "more than 150 s before the latest");
    assert!(v3::receive_non_authoritative(&trap_from(&sender, &keys, 3, 5000), &mut receiver).is_err(), "older boots");
    assert!(v3::receive_non_authoritative(&trap_from(&sender, &keys, 5, 1), &mut receiver).is_ok(), "a reboot");
    assert_eq!((receiver.engine_boots(), receiver.engine_time()), (5, 1));
}

fn report_of(bytes: &[u8]) -> (Header<'_>, String, i32) {
    let header = Header::parse(bytes).unwrap();
    assert!(!header.is_reportable());
    // An unauthenticated Report parses without keys; for an authenticated one only the
    // header is looked at here.
    let mut anyone = Security::new(header.user_name, b"").with_auth(Auth::NoAuthNoPriv);
    let (varbinds, req_id) = match v3::receive_non_authoritative(bytes, &mut anyone) {
        Ok(msg) => {
            assert_eq!(msg.pdu.message_type, MessageType::Report);
            (values_of(&msg.pdu).join(","), msg.pdu.req_id)
        }
        Err(_) => (String::new(), -1),
    };
    (header, varbinds, req_id)
}

#[test]
fn a_discovery_probe_is_answered_with_the_engine_id() {
    let engine_id = unhex("8000000005746465646f74");
    let mut local = LocalEngine::new(&engine_id, 7).unwrap().with_engine_time(99);
    let probe = unhex(INFORM_PROBE);
    let refusal = local.receive(&probe, None).unwrap_err();
    assert_eq!(refusal.error, Error::AuthFailure(AuthErrorKind::EngineIdMismatch));
    let report = refusal.report.expect("the probe is reportable");
    let (header, varbinds, req_id) = report_of(&report);
    let probe_header = Header::parse(&probe).unwrap();
    assert_eq!(header.msg_id, probe_header.msg_id);
    assert_eq!((header.engine_id, header.engine_boots), (&engine_id[..], 7));
    assert!((99..=100).contains(&header.engine_time));
    assert_eq!(header.security_level(), SecurityLevel::NoAuthNoPriv);
    assert_eq!(varbinds, "1.3.6.1.6.3.15.1.1.4.0=COUNTER32: 1");
    assert_eq!(req_id, 0x46e2c542, "the probe's request-id");
    assert_eq!(local.stats().unknown_engine_ids, 1);

    // A trap is not reportable: refused without a Report.
    let refusal = local.receive(&unhex(TRAP_SHA_AES), None).unwrap_err();
    assert_eq!(refusal.report, None);
}

/// TEDGE-DOT-PATCH(8): an unauthenticated notification against a user configured with
/// authentication is refused BEFORE it can teach anything.
///
/// The security state has to be adopted before the HMAC can be checked (the keys are localized
/// to the engine ID the message claims), and `verify()` runs only from AuthNoPriv up. So a
/// noAuthNoPriv datagram used to reach the learning branch, replace the engine ID, re-derive
/// the keys and return `Ok` — past the rollback — leaving every genuine authenticated trap from
/// that device failing `EngineIdMismatch` until a restart. The user name it needs is in the
/// clear in every v3 message, so nothing secret is required to send one.
#[test]
fn an_unauthenticated_notification_teaches_an_authenticated_user_nothing() {
    let configured = trap_user(Cipher::Aes128, AuthProtocol::Sha1);
    let engine_id = unhex("8000000005746465646f74");

    // A noAuthNoPriv message: no auth, no priv, carrying an engine ID of the sender's choosing.
    let mut unauthenticated_keys = Security::new(configured.username(), b"")
        .with_auth(Auth::NoAuthNoPriv)
        .with_engine_id(&engine_id)
        .unwrap();
    let name = oid(&[1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0]);
    let trap = oid(&[1, 3, 6, 1, 6, 3, 1, 1, 5, 3]);
    let datagram = v3::encode(
        &unauthenticated_keys,
        &Outgoing {
            msg_id: 7,
            level: SecurityLevel::NoAuthNoPriv,
            reportable: false,
            engine_id: &engine_id,
            engine_boots: 1,
            engine_time: 1,
            context_engine_id: &engine_id,
            context_name: b"",
        },
        MessageType::Trap,
        99,
        0,
        0,
        &[(&name, Value::ObjectIdentifier(trap))],
    )
    .unwrap();
    let _ = &mut unauthenticated_keys;

    // The receiver: the same user name, configured authPriv, with no engine ID pinned yet.
    let mut receiver = configured.clone();
    assert!(receiver.engine_id().is_empty(), "the fixture must start unpinned");

    let err = v3::receive_non_authoritative(&datagram, &mut receiver)
        .expect_err("an unauthenticated notification must be refused");
    assert_eq!(err, Error::AuthFailure(AuthErrorKind::UnsupportedSecLevel));
    assert!(
        receiver.engine_id().is_empty(),
        "a refused message must not teach the engine ID"
    );
}

/// The inform exchange of RFC 3414 §4 end to end: discovery, time synchronisation, the inform
/// and its Response — with the sender played by `encode` and `receive_non_authoritative`.
#[test]
fn an_inform_is_discovered_synchronised_and_acknowledged() {
    let engine_id = unhex("8000000005746465646f74");
    let mut local = LocalEngine::new(&engine_id, 2).unwrap().with_engine_time(500);
    for (cipher, level) in [(Cipher::Aes128, SecurityLevel::AuthPriv), (Cipher::Des, SecurityLevel::AuthNoPriv)] {
        let configured = trap_user(cipher, AuthProtocol::Sha1);
        let mut receiver_keys = local.localize(&configured).unwrap();
        // The sender learned the engine ID (see the probe test) but not boots/time yet.
        let sender_keys = local.localize(&configured).unwrap();
        let name = oid(&[1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0]);
        let trap = oid(&[1, 3, 6, 1, 4, 1, 99999, 0, 2]);
        let payload = oid(&[1, 3, 6, 1, 4, 1, 99999, 2, 1]);
        let inform = |boots, time| {
            v3::encode(
                &sender_keys,
                &Outgoing {
                    msg_id: 41,
                    level,
                    reportable: true,
                    engine_id: &engine_id,
                    engine_boots: boots,
                    engine_time: time,
                    context_engine_id: &engine_id,
                    context_name: b"ctx",
                },
                MessageType::InformRequest,
                1234,
                0,
                0,
                &[(&name, Value::ObjectIdentifier(trap.clone())), (&payload, Value::OctetString(b"inform me"))],
            )
            .unwrap()
        };

        let unsynchronised = inform(0, 0);
        let refusal = local.receive(&unsynchronised, Some(&mut receiver_keys)).unwrap_err();
        assert_eq!(refusal.error, Error::AuthFailure(AuthErrorKind::EngineTimeMismatch));
        let report = refusal.report.unwrap();
        let mut sender_view = local.localize(&configured).unwrap();
        let report_msg = v3::receive_non_authoritative(&report, &mut sender_view).expect("an authenticated Report");
        assert_eq!(report_msg.header.security_level(), SecurityLevel::AuthNoPriv);
        assert_eq!(values_of(&report_msg.pdu), ["1.3.6.1.6.3.15.1.1.2.0=COUNTER32: 1".replace(": 1", &format!(": {}", local.stats().not_in_time_windows))]);
        let (boots, time) = (report_msg.header.engine_boots, report_msg.header.engine_time);
        drop(report_msg);

        let datagram = inform(boots, time);
        let msg = local.receive(&datagram, Some(&mut receiver_keys)).expect("synchronised inform");
        assert_eq!(msg.pdu.message_type, MessageType::InformRequest);
        assert_eq!(msg.context_name, b"ctx");
        assert_eq!(values_of(&msg.pdu)[1], "1.3.6.1.4.1.99999.2.1=OCTET STRING: inform me");
        let ack = local.acknowledge(&msg).unwrap();
        drop(msg);

        let mut sender_view = local.localize(&configured).unwrap();
        let response = v3::receive_non_authoritative(&ack, &mut sender_view).unwrap();
        assert_eq!(response.pdu.message_type, MessageType::Response);
        assert_eq!((response.pdu.req_id, response.pdu.error_status, response.pdu.error_index), (1234, 0, 0));
        assert_eq!(response.header.msg_id, 41);
        assert_eq!(response.header.security_level(), level);
        assert_eq!(response.context_name, b"ctx");
        assert_eq!(values_of(&response.pdu).len(), 2);
    }
}

#[test]
fn refusals_carry_the_matching_report() {
    let engine_id = unhex("8000000005746465646f74");
    let mut local = LocalEngine::new(&engine_id, 1).unwrap();
    let configured = Security::new(b"informer", b"authpassword").with_auth_protocol(AuthProtocol::Sha256);
    let keys = local.localize(&configured).unwrap();
    let request = |user: &Security, level| {
        v3::encode(
            user,
            &Outgoing {
                msg_id: 3,
                level,
                reportable: true,
                engine_id: &engine_id,
                engine_boots: 1,
                engine_time: 0,
                context_engine_id: &engine_id,
                context_name: b"",
            },
            MessageType::InformRequest,
            8,
            0,
            0,
            &[],
        )
        .unwrap()
    };

    // Unknown user.
    let refusal = local.receive(&request(&keys, SecurityLevel::AuthNoPriv), None).unwrap_err();
    assert_eq!(report_of(&refusal.report.unwrap()).1, "1.3.6.1.6.3.15.1.1.3.0=COUNTER32: 1");

    // Wrong key.
    let wrong = local.localize(&Security::new(b"informer", b"otherpassword").with_auth_protocol(AuthProtocol::Sha256)).unwrap();
    let mut receiver_keys = keys.clone();
    let refusal = local.receive(&request(&wrong, SecurityLevel::AuthNoPriv), Some(&mut receiver_keys)).unwrap_err();
    assert_eq!(refusal.error, Error::AuthFailure(AuthErrorKind::SignatureMismatch));
    assert_eq!(report_of(&refusal.report.unwrap()).1, "1.3.6.1.6.3.15.1.1.5.0=COUNTER32: 1");

    // A level the user has no keys for.
    let mut no_auth = Security::new(b"informer", b"").with_auth(Auth::NoAuthNoPriv);
    let refusal = local.receive(&request(&keys, SecurityLevel::AuthNoPriv), Some(&mut no_auth)).unwrap_err();
    assert_eq!(report_of(&refusal.report.unwrap()).1, "1.3.6.1.6.3.15.1.1.1.0=COUNTER32: 1");

    // The right key, in time: accepted.
    let mut receiver_keys = keys.clone();
    assert!(local.receive(&request(&keys, SecurityLevel::AuthNoPriv), Some(&mut receiver_keys)).is_ok());
    // noAuthNoPriv from a user with keys is USM-valid; enforcing a minimum level is the caller's.
    let mut receiver_keys = keys.clone();
    let unauthenticated = request(&keys, SecurityLevel::NoAuthNoPriv);
    let msg = local.receive(&unauthenticated, Some(&mut receiver_keys)).unwrap();
    assert_eq!(msg.header.security_level(), SecurityLevel::NoAuthNoPriv);
}

#[test]
fn headers_are_validated() {
    assert!(Header::parse(&unhex(INFORM_PROBE)).is_ok());
    assert!(LocalEngine::new(&[1, 2, 3, 4], 0).is_err(), "engine IDs are 5 to 32 octets");
    assert!(LocalEngine::new(&[1, 2, 3, 4, 5], i64::from(i32::MAX)).is_err());
    let mut v2c = unhex(INFORM_PROBE);
    v2c[4] = 1;
    assert_eq!(Header::parse(&v2c).unwrap_err(), Error::UnsupportedVersion);
    // msgFlags privacy without authentication.
    let mut flags = unhex(INFORM_PROBE);
    let at = flags.windows(3).position(|w| w == [0x04, 0x01, 0x04]).unwrap();
    flags[at + 2] = 0x06;
    assert!(Header::parse(&flags).is_err());
    // Every prefix of a valid message is refused, and nothing panics on the way.
    let full = unhex(TRAP_SHA_AES);
    let mut user = trap_user(Cipher::Aes128, AuthProtocol::Sha1);
    let mut local = LocalEngine::new(&unhex(TRAP_ENGINE), 1).unwrap();
    for len in 0..full.len() {
        assert!(Header::parse(&full[..len]).is_err(), "prefix of {len}");
        assert!(v3::receive_non_authoritative(&full[..len], &mut user).is_err());
        assert!(local.receive(&full[..len], None).is_err());
    }
}

// ─── 6: requests carry the current engine time ───────────────────────────────────────────────

#[test]
fn a_request_carries_the_estimated_engine_time() {
    let engine_id = unhex("80001f8804746573742d61");
    let Some(learned_at) = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(200)) else {
        return;
    };
    let mut client = agent_security(&engine_id).with_engine_boots_and_time(3, 1000);
    client.authoritative_state.start_time = learned_at;
    let mut buf = pdu::Buf::default();
    pdu::build_get(Version::V3, b"", 1, &oid(&[1, 3, 6, 1]), &mut buf, Some(&client)).unwrap();
    let header = Header::parse(&buf).unwrap();
    assert_eq!(header.engine_boots, 3);
    assert!((1200..=1201).contains(&header.engine_time), "{}", header.engine_time);
    // The agent's view: in its time window, and the AES IV built from the same values.
    let mut agent = LocalEngine::new(&engine_id, 3).unwrap().with_engine_time(1200);
    let mut keys = agent.localize(&agent_security(&engine_id)).unwrap();
    let msg = agent.receive(&buf, Some(&mut keys)).expect("in the time window");
    assert_eq!(msg.pdu.message_type, MessageType::GetRequest);
}
