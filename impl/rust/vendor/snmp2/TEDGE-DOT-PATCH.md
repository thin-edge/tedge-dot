# Vendored `snmp2` 0.5.2 with patches

Copy of the crates.io package (MIT OR Apache-2.0), wired in through `[patch.crates-io]` in the
workspace `Cargo.toml`. Removed from the package: `.github/`, `.travis.yml`, `.gitignore`,
`justfile` and `rust-toolchain.toml` (it pins 1.90.0, and would switch toolchains for anyone
running cargo in this directory). The dev-dependency on tokio gains the features the new tests
need. Every change in `src/` carries a `TEDGE-DOT-PATCH(n)` comment naming the patch below;
their tests are in `src/tedge_dot_tests.rs` (`cargo test` in this directory).

tedge-dot uses it with `default-features = false, features = ["tokio", "crypto-rust",
"heap_buffers"]`: pure-Rust crypto (no OpenSSL to cross-compile), and the 65 KiB send/receive
buffers on the heap instead of inside every `AsyncSession` and on the stack of every v3 encode.

## 1. A malformed varbind list rejects the message

`impl Iterator for Varbinds` returns `None` at the first varbind that does not decode, so a
truncated or corrupt list was indistinguishable from its end: `Pdu::from_bytes` accepted the
message and the caller saw fewer varbinds. Values of a type the crate has no variant for (any
primitive tag it does not know, e.g. an SMIv1 `UInteger32` 0x47) and every BOOLEAN (the reader
compared the tag against NULL) had the same effect.

- `AsnReader::read_value()` — the value decoder behind the iterator, returning the error.
- `Value::Unknown(tag, content)` for unknown primitive tags; multi-octet tags stay errors.
- `Varbinds::check()` walks the whole list; every `Pdu` constructor (v1/v2c, v1 Trap, v3) calls
  it, so the iterator can no longer end early on a `Pdu`.
- `read_asn_boolean` checks for the BOOLEAN tag.
- `AsnReader::remaining()` / `Varbinds::as_bytes()`, and `push_varbinds` encodes `Unknown`,
  `Sequence`, `Set` and `Constructed` values (it silently emitted an empty varbind), so a
  received list can be echoed (an inform's Response).

Upstream draft: `doc/upstream/snmp2-varbind-truncation.md`.

## 2. Value ranges and OID validity

- Counter32, Gauge32/Unsigned32 and TimeTicks were read as `i64` and cast with `as u32`, so
  2^32 arrived as 0; a Counter64 of 2^63 or more (nine content octets, as net-snmp sends
  `0xFFFFFFFFFFFFFFFF`) failed to decode at all. They are now decoded as unsigned, at most one
  octet longer than the type needs, and out-of-range values are errors.
- `push_counter64` encoded `n as i64`: values from 2^63 went out as negative INTEGERs.
- An INTEGER with no content octets decoded as 0 through a shift by 64 (a panic in debug
  builds); it is an error.
- `read_asn_objectidentifier` stored any octets. It now rejects an empty OID, a final octet
  with the continuation bit set, and sub-identifiers above 32 bits (SMI's limit).

Upstream draft: `doc/upstream/snmp2-value-ranges-and-oids.md`.

## 3. No write through the shared input buffer

`Pdu::parse_v3` zeroed msgAuthenticationParameters in place before computing the HMAC, with
`unsafe { ptr::write_bytes(bytes.as_ptr().add(..).cast_mut(), ..) }` on the caller's `&[u8]`:
undefined behaviour, a visible mutation of the caller's datagram, and a crash for input in
read-only memory. The HMAC is computed over a copy.

Upstream draft: `doc/upstream/snmp2-v3-auth-params-ub.md`.

## 4. Stale responses after a timeout

`AsyncSession` has no request timeout (a lost datagram makes `recv` wait forever), so callers
wrap requests in `tokio::time::timeout`. The late response to an abandoned request then stayed
in the socket and was read as the answer to the next one, failing it with `RequestIdMismatch`
— and each late datagram could fail one more request. The request methods now skip datagrams
that parse as a response to a different request-id and keep waiting for their own. The
timeout itself stays the caller's (tedge-dot applies `request_timeout` and `retries`).

Upstream draft: `doc/upstream/snmp2-async-timeout-stale-responses.md`.

## 5. Receiver-side SNMPv3 (`v3::receiver`)

The crate implements USM for a command generator only. A notification receiver also needs:

- **the authoritative role** (`LocalEngine`): an InformRequest is sent under the receiver's
  engine ID, boots and time. `LocalEngine::receive` performs RFC 3414 §3.2 as the authoritative
  engine and, for a reportable message it refuses, builds the Report the sender expects
  (`usmStatsUnknownEngineIDs` for a discovery probe, `usmStatsNotInTimeWindows` — authenticated
  — for time synchronisation, `usmStatsUnknownUserNames`, `usmStatsUnsupportedSecLevels`,
  `usmStatsWrongDigests`, `usmStatsDecryptionErrors`). `LocalEngine::acknowledge` / `respond`
  build the Response under the local engine in the request's context. `localize` derives a
  user's keys for the local engine.
- **the non-authoritative role for traps** (`receive_non_authoritative`): upstream's parser
  rejected every trap with msgAuthoritativeEngineBoots 0 (`EngineBootsNotProvided`), which is
  what net-snmp's `snmptrap` sends; learned the sender's engine ID from unauthenticated
  messages and kept it after a failed check; and never checked timeliness for traps. The new
  function checks user, level, HMAC and the latest boots/time of the sender (§3.2 step 7b,
  replay protection), learns the engine ID only from a message that passes, and leaves the
  security state untouched on any failure.
- `Header::parse` — msgGlobalData and the USM parameters, to pick the user and the role before
  any key is involved; `SecurityLevel`; `Security::security_level()`.
- `v3::encode` — a v3 message from explicit msgID, flags, engine ID/boots/time and context
  (upstream always used msgID = request-id, the reportable flag, and the security's own engine).
- AES encryption/decryption take the boots/time the message carries (upstream used the
  security state's, which differ for the authoritative role).
- DES decryption no longer strips "PKCS#7 padding": RFC 3414 §8.1.1.2 leaves the padding
  octets unspecified, net-snmp pads with zeros, and upstream rejected those messages.
- `pdu::encode` — a v1/v2c message of any PDU type (responses, traps, informs), and
  `MessageType::ident()`.
- `AsyncSession::security()` — the v3 state a session discovered, so the engine ID a successful
  request learned can be held against the one a trap claims.
- `AuthErrorKind::UnsupportedSecLevel`.

Upstream draft: `doc/upstream/snmp2-receiver-side-usm.md`.

## 6. v3 requests carry the current engine time

Requests sent the authoritative engine time learned at discovery or from the last response,
unchanged (`engine_time_current` was computed and never used). A request more than 150 s after
the previous exchange fell outside the agent's time window: the agent answered with a
`notInTimeWindows` Report, which the client then rejected as `EngineTimeMismatch` — so a v3
device polled less often than every 150 s failed every poll. Requests now carry the learned
time plus the local time elapsed since.

Upstream draft: `doc/upstream/snmp2-v3-stale-engine-time.md`.

Drop this directory and the `[patch.crates-io]` entry once a release carries equivalent fixes.
