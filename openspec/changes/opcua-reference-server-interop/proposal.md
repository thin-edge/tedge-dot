# Proposal

## Why

Both OPC UA connectors are only ever tested against servers we wrote ourselves: the
python-`asyncua` simulator (`connectors/opcua/sim/server.py`) and the embedded `async-opcua`
server in the conformance harness. Nothing proves either connector interoperates with a
third-party stack, and the two we use share our own reading of the specification — so a
misreading is invisible to every test we have. Half of the security surface we shipped in
`opcua-secure-connections` has never been exercised on the wire: `Aes128_Sha256_RsaOaep`
appears only in a negative "no matching endpoint" test, `Basic128Rsa15` and `Basic256` only in
config-validation unit tests, and secure-channel token renewal is not tested at all.

The OPC Foundation's UA-.NETStandard reference stack is the implementation most real servers
are built on or validated against. `php-opcua/uanetstandard-test-suite` packages it as ~13
server instances covering the full policy/mode/authentication matrix, under MIT (both the
wrapper and UA-.NETStandard's OPC Foundation MIT 1.00 — no RCL, no membership clause), so it
fits the project's OSS-only constraint and can be used as an interop target in CI.

## What Changes

- A new **interop test suite** runs both connector implementations against the
  UA-.NETStandard reference servers, in its own compose stack and its own CI job. It is
  additive: `sim/server.py` remains the default e2e server and the sole source of the negative
  PKI vectors (revoked, expired, wrong host, missing CRL, foreign CA), which the reference
  server cannot be made to present without forking it.
- Coverage the current suites do not have, now proven against a third-party stack:
  - every supported policy on the wire, including `Aes128_Sha256_RsaOaep` positively and
    `Basic128Rsa15`/`Basic256` behind `allow_deprecated_security`
  - `sign` without encryption, against a server that offers only `Sign`
  - username and X.509 user identities accepted and rejected by a real server
  - an endpoint with a non-empty resource path (`/UA/TestServer`) and one with none
  - secure-channel token renewal against a server with a 30-second token lifetime
  - a server that does **not** auto-accept our application certificate
  - an ECC-only server, which both connectors must refuse cleanly
- **Fix**: a server that rejects *our* application certificate is currently reported as
  `certificate untrusted:` / "the server certificate is not trusted"
  (`impl/rust/crates/connector-opcua/src/security.rs:25`, mirrored in C). That is backwards and
  points the operator at the wrong trust store. It gains its own reason category naming the
  real remedy — export our certificate to the server administrator.
- **New requirement**: the connector renews the secure-channel token before it expires and the
  session survives; today nothing specifies or tests this.
- **Clarification**: the `no matching endpoint:` reason must list policies the server offers
  that the connector does not implement (the ECC ones) rather than dropping them, so the
  reason explains why nothing matched.
- **Clarification**: endpoint selection preserves the configured endpoint's resource path, not
  just its host and port.
- Not in scope: implementing ECC policies, PubSub/UADP, HTTPS transport, reverse connect, SKS,
  and replacing either existing simulator.

## Capabilities

### New Capabilities
- `opcua-reference-server-interop`: what must hold when either connector talks to the OPC
  Foundation UA-.NETStandard reference server — the policy/mode/identity matrix it must
  cover, how trust is bootstrapped against a server whose certificate is regenerated on every
  start, how the suite is gated and pinned, and the parity requirement across both
  implementations.

### Modified Capabilities
- `opcua-secure-connections`: adds a reason category for the server rejecting our application
  certificate; adds secure-channel token renewal; clarifies that unimplemented policies are
  still listed in the `no matching endpoint:` reason and that endpoint selection keeps the
  configured resource path.

## Impact

- **Code**: `impl/rust/crates/connector-opcua/src/security.rs` (reason categories, endpoint
  selection), `impl/c/connectors/opcua/connector_opcua.c` (the same mapping), and whatever
  each stack needs for token renewal.
- **Tests / CI**: a new compose stack and Robot suite under `connectors/opcua/`, a
  `just test-interop opcua` recipe plus its `-c` variant, and a new CI job. The image is
  `linux/amd64` only, so the job runs on amd64 runners and the local recipe is opt-in rather
  than part of `just test-e2e opcua`.
- **Dependencies**: a pinned `ghcr.io/php-opcua/uanetstandard-test-suite` tag, pulled at test
  time only — nothing is linked into either connector and nothing ships in a package.
- **Docs**: `doc/connectors/opcua-connector-spec.md` §10 gains the interop layer.
- **Risk**: the upstream project is young and single-maintainer, with known doc/code
  mismatches; pinning the tag and keeping our own simulator as the default e2e server contains
  that.
