# Upstream issue draft: snmp2 narrows 32-bit values, rejects large Counter64s and stores invalid OIDs

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

In `src/asn1.rs`:

- `read_snmp_counter32`, `read_snmp_unsigned32` and `read_snmp_timeticks` read an `i64` and cast
  it with `as u32`. A value above 2^32−1 is silently narrowed (`41 05 01 00 00 00 00`, 2^32,
  decodes as `Counter32(0)`), where it should be an error.
- `read_snmp_counter64` reads an `i64` and casts it. `decode_i64` refuses more than eight
  content octets, but a Counter64 of 2^63 or more needs nine (a leading zero): net-snmp's
  `snmptrap ... c 18446744073709551615` sends `46 09 00 ff ff ff ff ff ff ff ff`, which fails —
  and ends the varbind list silently (see the truncation issue).
- `decode_i64` accepts zero content octets and computes `(ret << 64) >> 64`: a panic in debug
  builds, 0 in release. BER requires at least one octet.
- `read_asn_objectidentifier` wraps any octets in `Oid::new`: an empty OID, one ending with a
  continuation bit (`2b 86`) and sub-identifiers beyond 32 bits (`2b 90 80 80 80 00`) all parse,
  and only fail (or print oddly) when a caller iterates the arcs.
- The encoder has the mirror problem: `push_counter64(n)` calls `push_i64(n as i64)`, so every
  value from 2^63 is sent as a negative INTEGER (`u64::MAX` as `46 01 ff`).

## Reproduction

```rust
let trap = /* v2c trap with a varbind 1.3.6.1 = Counter32 with content 01 00 00 00 00 */;
let pdu = snmp2::Pdu::from_bytes(&trap).unwrap();
println!("{:?}", pdu.varbinds);   // COUNTER32: 0

let big = /* ... Counter64 with content 00 ff ff ff ff ff ff ff ff */;
let pdu = snmp2::Pdu::from_bytes(&big).unwrap();
assert_eq!(pdu.varbinds.count(), 0); // the varbind disappeared
```

## Proposed fix

- Decode the unsigned types as unsigned: content of 1 to (octets the type needs + 1) octets,
  big-endian, rejected above the type's maximum (`u32::MAX`, `u64::MAX`).
- Reject an INTEGER with no content octets (`AsnInvalidLen`).
- Validate OBJECT IDENTIFIER content when reading it: non-empty, last octet without the
  continuation bit, every sub-identifier ≤ 2^32−1 (RFC 2578 §7.1.3).
- Encode Counter64 as a non-negative value: minimal big-endian octets with a leading zero when
  the top bit is set.

## Local workaround

tedge-dot vendors snmp2 with this change (`impl/rust/vendor/snmp2`, patch 2). Found on
2026-09-15 with the SNMP connector's golden vectors.
