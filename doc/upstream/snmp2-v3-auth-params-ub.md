# Upstream issue draft: snmp2 writes through a shared slice when verifying SNMPv3 authentication

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

`Pdu::parse_v3` (`src/v3.rs`) must compute the HMAC over the message with
msgAuthenticationParameters zeroed. It does so in place, on the caller's input:

```rust
unsafe {
    let auth_params_ptr = bytes.as_ptr().add(auth_params_pos).cast_mut();
    // TODO: switch to safe code as the solution may be pretty fragile
    std::hint::black_box(|| {
        std::ptr::write_bytes(auth_params_ptr, 0, auth_params.len());
    })();
}
```

`bytes` is a `&'a [u8]`. Writing through a pointer derived from a shared reference is undefined
behaviour regardless of `black_box` (the optimizer may assume the memory is unchanged, and
`Pdu<'a>` borrows from the same buffer). Observable effects today:

- the caller's datagram is modified: after `Pdu::from_bytes_with_security(&buf, ..)` the
  authentication parameters in `buf` are zeros, so the message can no longer be re-verified,
  forwarded or logged as received;
- input in read-only memory (a `static` test vector, a memory-mapped capture) segfaults.

## Reproduction

```rust
let datagram: Vec<u8> = /* any authenticated v3 Response */;
let copy = datagram.clone();
let mut security = /* matching Security */;
let _ = snmp2::Pdu::from_bytes_with_security(&datagram, Some(&mut security));
assert_eq!(datagram, copy); // fails: 12 octets are now zero
```

Run under Miri for the UB report.

## Proposed fix

Sign a copy (the message is at most one datagram):

```rust
let mut signed = bytes.to_vec();
signed[auth_params_pos..auth_params_pos + auth_params.len()].fill(0);
let hmac = security.calculate_hmac(&signed)?;
```

Comparing the truncated HMAC in constant time (`hmac::Mac::verify_truncated_left`, or a XOR
fold) would also be preferable to `!=`.

## Local workaround

tedge-dot vendors snmp2 with the copy (`impl/rust/vendor/snmp2`, patch 3; test
`parsing_an_authenticated_message_leaves_the_input_untouched`). Found on 2026-09-15 while
reviewing the library for the SNMP connector.
