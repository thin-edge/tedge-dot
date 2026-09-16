# Upstream issue draft: snmp2 silently truncates a malformed varbind list

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

`Pdu::from_bytes` only records where the VarBindList is; `impl Iterator for Varbinds`
(`src/lib.rs`) decodes it lazily and returns `None` as soon as a varbind does not decode
(`if let Ok(seq) = ... { if let (Ok(name), Some(value)) = ... }` → `None`). A truncated or
corrupt list is therefore indistinguishable from the end of the list: the message is accepted
and the caller silently sees the varbinds before the damage. The same happens for perfectly
valid messages the value decoder does not support:

- a primitive tag with no `Value` variant (`AsnReader::next` maps it to
  `Err(AsnUnsupportedType)` and then `.ok()`), e.g. an SMIv1 `UInteger32` (0x47);
- every BOOLEAN: `AsnReader::read_asn_boolean` compares the tag against `TYPE_NULL`
  instead of `TYPE_BOOLEAN`, so it always fails;
- a Counter64 of 2^63 or more (see the value-ranges issue), which net-snmp sends for
  `0xFFFFFFFFFFFFFFFF`.

For a trap receiver this means a notification whose interesting varbind comes after one of
these is processed as if the varbind were absent.

## Reproduction

```rust
// v2c trap: snmpTrapOID.0 = 1.3.6.1, then a varbind with a name and no value.
let msg = hex("302702010104067075626c6963a71a020107020100020100300f300d06082b0601060301010401000601" /* ... */);
let pdu = snmp2::Pdu::from_bytes(&msg).unwrap();      // Ok
assert_eq!(pdu.varbinds.count(), 1);                   // the second varbind vanished
```

Any list whose second varbind is `SEQUENCE { OID }` (no value), is not a SEQUENCE, or holds a
value of tag 0x47 shows it. Also `pdu.varbinds` never yields a BOOLEAN.

## Proposed fix

1. Split the value decoder out of `impl Iterator for AsnReader` as
   `pub fn read_value(&mut self) -> Result<Value<'a>>` (the iterator keeps calling it).
2. Validate the list when a `Pdu` is built: a `Varbinds::check() -> Result<usize>` that walks
   every element (`SEQUENCE { OBJECT IDENTIFIER, value }`) and is called from `from_bytes_inner`,
   `parse_trap_v1` and `parse_v3`, so the iterator cannot end early on a `Pdu`.
3. Represent unknown primitive tags instead of failing: `Value::Unknown(u8, &'a [u8])` (tag and
   content octets); keep multi-octet tags (`tag & 0x1F == 0x1F`) an error. Adding a variant is a
   breaking change for exhaustive matches; `#[non_exhaustive]` on `Value` would avoid the next one.
4. Fix `read_asn_boolean` to compare against `TYPE_BOOLEAN`.
5. Let `push_varbinds` encode `Unknown`, `Sequence`, `Set` and `Constructed` values from their
   content (it currently `return`s from the closure and emits an empty varbind SEQUENCE), so a
   received list can be sent back — e.g. in the Response to an InformRequest.

## Local workaround

tedge-dot vendors snmp2 with this change (`impl/rust/vendor/snmp2`, patch 1 in its
`TEDGE-DOT-PATCH.md`, tests in `src/tedge_dot_tests.rs`). Found on 2026-09-15 while running the
SNMP connector's golden vectors (`connectors/snmp/conformance/trap-vectors.json`) through the
library: 26 of 43 malformed datagrams were accepted, the varbind cases among them.
