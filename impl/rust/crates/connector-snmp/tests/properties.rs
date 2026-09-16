//! Property tests over the decode path: whatever a well-formed v2c notification holds, encoding
//! then decoding it gives it back; no prefix of one decodes; no input panics; and the canonical
//! content octets a sample carries round-trip.

use connector_snmp::notification::{self, PduKind, Version};
use connector_snmp::snmp2::{pdu, MessageType, Oid as LibOid, Value as SnmpValue, Version as LibVersion};
use connector_snmp::value::{Oid, VarValue};
use proptest::prelude::*;

fn oid() -> impl Strategy<Value = Oid> {
    (0u32..=2, 0u32..40, prop::collection::vec(any::<u32>(), 0..12)).prop_map(
        |(first, second, rest)| {
            let mut arcs = vec![first, second];
            arcs.extend(rest);
            let dotted: Vec<String> = arcs.iter().map(u32::to_string).collect();
            Oid::parse(&dotted.join(".")).expect("generated OID is valid")
        },
    )
}

/// One varbind value, as the library models it (owned, so it can cross a strategy).
#[derive(Clone, Debug)]
enum Owned {
    Integer(i64),
    OctetString(Vec<u8>),
    Null,
    Oid(Oid),
    Ip([u8; 4]),
    Counter32(u32),
    Gauge32(u32),
    Timeticks(u32),
    Opaque(Vec<u8>),
    Counter64(u64),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
}

impl Owned {
    fn value(&self) -> SnmpValue<'_> {
        match self {
            Owned::Integer(v) => SnmpValue::Integer(*v),
            Owned::OctetString(b) => SnmpValue::OctetString(b),
            Owned::Null => SnmpValue::Null,
            Owned::Oid(oid) => SnmpValue::ObjectIdentifier(oid.to_library()),
            Owned::Ip(ip) => SnmpValue::IpAddress(*ip),
            Owned::Counter32(v) => SnmpValue::Counter32(*v),
            Owned::Gauge32(v) => SnmpValue::Unsigned32(*v),
            Owned::Timeticks(v) => SnmpValue::Timeticks(*v),
            Owned::Opaque(b) => SnmpValue::Opaque(b),
            Owned::Counter64(v) => SnmpValue::Counter64(*v),
            Owned::NoSuchObject => SnmpValue::NoSuchObject,
            Owned::NoSuchInstance => SnmpValue::NoSuchInstance,
            Owned::EndOfMibView => SnmpValue::EndOfMibView,
        }
    }
}

fn value() -> impl Strategy<Value = Owned> {
    prop_oneof![
        any::<i64>().prop_map(Owned::Integer),
        prop::collection::vec(any::<u8>(), 0..300).prop_map(Owned::OctetString),
        Just(Owned::Null),
        oid().prop_map(Owned::Oid),
        any::<[u8; 4]>().prop_map(Owned::Ip),
        any::<u32>().prop_map(Owned::Counter32),
        any::<u32>().prop_map(Owned::Gauge32),
        any::<u32>().prop_map(Owned::Timeticks),
        prop::collection::vec(any::<u8>(), 0..16).prop_map(Owned::Opaque),
        any::<u64>().prop_map(Owned::Counter64),
        Just(Owned::NoSuchObject),
        Just(Owned::NoSuchInstance),
        Just(Owned::EndOfMibView),
    ]
}

fn notification_parts() -> impl Strategy<Value = (PduKind, Vec<u8>, i32, Oid, Vec<(Oid, Owned)>)> {
    (
        prop_oneof![Just(PduKind::Trap), Just(PduKind::Inform)],
        prop::collection::vec(any::<u8>(), 0..32),
        any::<i32>(),
        oid(),
        prop::collection::vec((oid(), value()), 0..20),
    )
}

/// Encode a v2c notification: snmpTrapOID.0 first, then whatever the case carries.
fn encode(
    pdu_kind: PduKind,
    community: &[u8],
    request_id: i32,
    trap: &Oid,
    rest: &[(Oid, Owned)],
) -> (Vec<u8>, Vec<(Oid, Owned)>) {
    let trap_oid = Oid::parse("1.3.6.1.6.3.1.1.4.1.0").unwrap();
    let mut varbinds: Vec<(Oid, Owned)> = vec![(trap_oid, Owned::Oid(trap.clone()))];
    varbinds.extend_from_slice(rest);
    let names: Vec<LibOid<'static>> = varbinds.iter().map(|(oid, _)| oid.to_library()).collect();
    let values: Vec<(&LibOid<'static>, SnmpValue)> = names
        .iter()
        .zip(varbinds.iter())
        .map(|(name, (_, value))| (name, value.value()))
        .collect();
    let message_type = match pdu_kind {
        PduKind::Trap => MessageType::Trap,
        PduKind::Inform => MessageType::InformRequest,
    };
    let bytes = pdu::encode(LibVersion::V2C, community, message_type, request_id, 0, 0, &values)
        .expect("a v2c notification encodes");
    (bytes, varbinds)
}

proptest! {
    #[test]
    fn encode_then_decode_round_trips(
        (pdu_kind, community, request_id, trap, rest) in notification_parts()
    ) {
        let (message, varbinds) = encode(pdu_kind, &community, request_id, &trap, &rest);
        let n = notification::decode(&message).expect("an encoded notification decodes");
        prop_assert_eq!(n.pdu, pdu_kind);
        prop_assert_eq!(&n.community, &community);
        prop_assert_eq!(n.request_id, Some(i64::from(request_id)));
        prop_assert_eq!(&n.trap, &trap);
        prop_assert_eq!(n.varbinds.len(), varbinds.len());
        for (got, (name, _)) in n.varbinds.iter().zip(&varbinds) {
            prop_assert_eq!(&got.name, name);
        }
        prop_assert_eq!(n.version, Version::V2c);
    }

    #[test]
    fn no_strict_prefix_of_a_notification_decodes(
        (pdu_kind, community, request_id, trap, rest) in notification_parts(),
        cut in any::<prop::sample::Index>()
    ) {
        let (message, _) = encode(pdu_kind, &community, request_id, &trap, &rest);
        let len = cut.index(message.len());
        prop_assert!(notification::decode(&message[..len]).is_err());
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(n) = notification::decode(&bytes) {
            let _ = n.trap.encode();
            let _ = n.trap_address();
        }
    }

    #[test]
    fn a_values_canonical_octets_decode_back_to_it(owned in value()) {
        let value = VarValue::from_library(&owned.value()).expect("a valid value");
        // The canonical content octets are what a sample's `raw` carries; re-reading them as the
        // same type gives the same value back.
        let mut element = vec![match &owned {
            Owned::Integer(_) => 0x02,
            Owned::OctetString(_) => 0x04,
            Owned::Null => 0x05,
            Owned::Oid(_) => 0x06,
            Owned::Ip(_) => 0x40,
            Owned::Counter32(_) => 0x41,
            Owned::Gauge32(_) => 0x42,
            Owned::Timeticks(_) => 0x43,
            Owned::Opaque(_) => 0x44,
            Owned::Counter64(_) => 0x46,
            Owned::NoSuchObject => 0x80,
            Owned::NoSuchInstance => 0x81,
            Owned::EndOfMibView => 0x82,
        }];
        let raw = &value.raw;
        if raw.len() < 0x80 {
            element.push(raw.len() as u8);
        } else {
            element.push(0x82);
            element.extend((raw.len() as u16).to_be_bytes());
        }
        element.extend_from_slice(raw);
        let again = connector_snmp::snmp2::AsnReader::from_bytes(&element)
            .read_value()
            .expect("canonical octets decode");
        prop_assert_eq!(VarValue::from_library(&again).unwrap(), value);
    }

    #[test]
    fn canonical_oid_encoding_round_trips(o in oid()) {
        prop_assert_eq!(Oid::from_ber(&o.encode()).unwrap(), o.clone());
        prop_assert_eq!(Oid::parse(&o.to_string()).unwrap(), o);
    }
}
