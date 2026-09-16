# Upstream feature draft: snmp2 receiver-side SNMPv3 (informs, discovery Reports, traps)

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

snmp2's USM implementation assumes the local engine is always a command generator talking to
an authoritative agent. A notification receiver cannot be built on it for v3:

1. **Informs are impossible.** An InformRequest is addressed to the *receiver's* engine ID,
   boots and time (RFC 3414 §4). The receiver must own an engine identity, answer a sender that
   has not discovered it with a Report (`usmStatsUnknownEngineIDs`, unauthenticated, carrying
   the engine ID; then `usmStatsNotInTimeWindows`, authenticated, carrying boots and time), check
   timeliness against its own clock, and send the Response under its own engine. None of this
   exists: `Security` always describes the remote engine, `v3::build` always uses
   msgID = request-id, sets the reportable flag, and uses the security's engine as the
   context engine.
2. **Traps from net-snmp are rejected.** `parse_v3` fails any authenticated message with
   msgAuthoritativeEngineBoots 0 while the local state is 0 (`EngineBootsNotProvided`);
   `snmptrap -v 3` without persistent state sends boots 0.
3. **Engine ID learning is not tied to authentication.** With an empty engine ID the parser
   adopts the message's engine ID (and localizes keys) before verifying anything, and keeps it
   if verification then fails: one spoofed datagram pins a wrong engine ID.
4. **No replay protection for traps**: the timeliness check is skipped for `MessageType::Trap`
   (RFC 3414 §3.2 step 7b defines it for non-authoritative receivers too).
5. **AES IVs use the security state's boots/time**, not the message's — equal for a client, not
   for the roles above.
6. **DES messages from net-snmp fail to decrypt** with the pure-Rust backend: it strips
   "PKCS#7 padding" and rejects a zero final octet, but RFC 3414 §8.1.1.2 leaves the padding
   octets unspecified (the scoped PDU is self-delimiting) and net-snmp pads with zeros.

## Reproduction

- Capture `snmptrap -v 3 -e 0x8000000001020304 -u trapuser -l authPriv -a SHA -A authpassword -x
  AES -X privpassword host:162 0 1.3.6.1.6.3.1.1.5.3` and parse it with
  `Pdu::from_bytes_with_security` and a matching `Security` → `EngineBootsNotProvided`.
- Run `snmpinform -v 3 ...` against any receiver built on snmp2: the first datagram is a GET
  with an empty engine ID; there is no API to answer it.

## Proposed API (as implemented in the tedge-dot copy)

```rust
pub mod v3::receiver {
    pub struct Header<'a> { pub msg_id, pub max_size, pub flags, pub engine_id, pub engine_boots,
                            pub engine_time, pub user_name, /* private offsets */ }
    impl Header<'_> { fn parse(bytes) -> Result<Header>; fn security_level(); fn is_reportable(); }

    pub struct Message<'a> { pub header, pub context_engine_id, pub context_name, pub pdu, pub security }
    pub struct Refusal { pub error: Error, pub report: Option<Vec<u8>> }

    pub struct LocalEngine { /* engine ID, boots, clock, usmStats */ }
    impl LocalEngine {
        fn new(engine_id, boots) -> Result<Self>;
        fn localize(&self, user: &Security) -> Result<Security>;
        fn receive<'a>(&mut self, bytes: &'a [u8], user: Option<&'a mut Security>)
            -> Result<Message<'a>, Refusal>;          // RFC 3414 §3.2 as authoritative engine
        fn respond(&self, request: &Message, type, error_status, error_index, varbinds) -> Result<Vec<u8>>;
        fn acknowledge(&self, inform: &Message) -> Result<Vec<u8>>;
    }

    /// Trap / Response / Report under the sender's engine; state untouched on failure.
    pub fn receive_non_authoritative<'a>(bytes: &'a [u8], security: &'a mut Security) -> Result<Message<'a>>;

    pub struct Outgoing<'b> { msg_id, level, reportable, engine_id, engine_boots, engine_time,
                              context_engine_id, context_name }
    pub fn encode(user: &Security, out: &Outgoing, type, req_id, error_status, error_index, varbinds) -> Result<Vec<u8>>;
}
pub fn pdu::encode(version, community, type, req_id, error_status, error_index, varbinds) -> Result<Vec<u8>>; // v1/v2c
```

`LocalEngine::receive` refuses in RFC order — unknown engine ID, unknown user, unsupported level,
wrong digest, not in time window, decryption error — and builds a Report only when the
message's reportable flag is set. The same engine type serves an SNMPv3 agent answering
requests, which is how the tedge-dot tests run an in-process v3 agent.

Supporting changes: `Security::{encrypt_with, decrypt_with}` taking the boots/time of the
message; DES decryption keeping the padding; `AuthErrorKind::UnsupportedSecLevel`.

## Local workaround

tedge-dot vendors snmp2 with this module (`impl/rust/vendor/snmp2/src/v3/receiver.rs`, patch
5; tests in `src/tedge_dot_tests.rs`, including net-snmp 5.6 captures and a full
discovery → time sync → inform → Response exchange). Written on 2026-09-15 for the SNMP
connector's v3 trap and inform support.
