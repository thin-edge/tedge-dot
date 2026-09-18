# Tasks

## 1. Connector fixes exposed by this work

- [x] 1.1 Add the `application certificate rejected:` reason category to the Rust connector
      (`impl/rust/crates/connector-opcua/src/security.rs`): when the server-certificate
      pre-check passed or was skipped by `trust_any_server_certificate` and the handshake or
      activation then fails with `BadSecurityChecksFailed`/`BadCertificateUntrusted`, report
      the new category naming our own thumbprint and subject. Verify with a unit test in
      `tests/security.rs` covering both orderings (we distrust them → `certificate untrusted:`;
      they distrust us → the new category)
- [x] 1.2 Mirror 1.1 in the C connector (`impl/c/connectors/opcua/connector_opcua.c`) with
      byte-identical reason text, and verify `just c-pki-parity` and the `opcua-*` ctest
      targets still pass. NOTE: the C build has no in-process OPC UA server, so nothing
      locally exercises this branch — its behavioural proof is task 3.5, which runs the
      interop suite under both implementations
- [x] 1.3 Keep the configured endpoint's resource path when dialling a selected endpoint, in
      both implementations; verify with a unit test per implementation asserting that an
      advertised endpoint with a different host and path still yields the configured host,
      port and path
- [x] 1.4 Include policies the connector does not implement in the `no matching endpoint:`
      reason, named as the server advertised them, in both implementations; verify with a unit
      test feeding an endpoint list containing an unknown policy URI
- [x] 1.5 Confirm or implement secure-channel token renewal in both implementations: verify
      with a test that a session against a server granting a short token lifetime stays
      `connected` across at least two renewals with no reconnect logged. DONE: both client
      stacks already renew in their transport layer (async-opcua
      `should_renew_security_token`, at 75% of the granted lifetime; open62541 renews in its
      housekeeping). `session_survives_secure_channel_token_renewal` in `tests/security.rs`
      pins it for Rust against a server clamped to a 2s token — four renewals observed in one
      run. C is covered behaviourally by task 3.8

- [x] 1.6 FOUND BY THE INTEROP SUITE: a server certificate that fails the key-length check is
      reported as `certificate untrusted:` and the reason tells the operator to run
      `tedge-dot pki trust` -- which they may already have done, and which cannot help. The
      UA-.NETStandard server's auto-generated certificate is RSA-1024 (upstream's own comment
      in `Program.cs`), so trusting it changes nothing: async-opcua logs "invalid key length
      1024 for the policy Basic256Sha256" but returns a generic `BadCertificateUntrusted`,
      which we map to the wrong category. Check the key length against the effective policy
      ourselves before consulting the trust store, and report it as `certificate invalid:`
      naming the key size and the policy. DONE IN RUST ONLY. (An earlier reading of `ua_pki.c` suggested C already
      handled this because `ua_pki_validate` can return
      `BADCERTIFICATEPOLICYCHECKFAILED`; the interop run disproved that -- C checks the key
      length only *after* the trust lookup, so it reports the same misleading
      `certificate untrusted: ... trust it with pki trust`. See task 1.7.) `Policy::key_bits()` mirrors
      the C `POLICIES` table and is checked before the trust store. Proven by
      `Weak Server Certificate Is Refused With Its Key Length` against the real server. A
      certificate refused this way is no longer quarantined, which is correct: `rejected/` is
      a list of certificates an operator could choose to trust, and trusting this one could
      never help. Bounds pinned by `policy_key_bits_match_the_c_table`.

- [x] 1.7 FOUND BY THE C INTEROP RUN -- three parity gaps, none of them reproducible against
      this project's own simulators. All three are fixed; `just test-interop-c opcua` went
      from 4/12 to 11/13 on the first fix alone.
      a) **No secured session to the reference server succeeded at all.** ROOT CAUSE: not a C
         defect. open62541 enforces the security policy's RSA key range inside the policy
         itself (`securitypolicy_basic256sha256.c` returns
         `BadCertificateUseNotAllowed`), where no trust setting can reach it -- correctly, as
         OPC UA Part 7 makes the range part of the policy. The reference server
         auto-generates a 1024-bit certificate. **Rust was the lax one**: its key-length check
         sat after the `trust_any_server_certificate` early return, so that option silently
         waived a policy requirement too. Fixed by moving the check ahead of both the trust
         store and the opt-out, and by giving the servers a real RSA-2048 certificate
         (`gen-server-certs.sh`) so the policy matrix is testable at all.
      b) **Key length was checked after the trust lookup in C as well**, so a weak certificate
         was quarantined and the reason advised `tedge-dot pki trust`. Fixed in
         `connector_opcua.c` with the same text as Rust.
      c) **`BadCertificateUseNotAllowed` was mis-categorised** -- which turned out to be (a)
         wearing a different hat: with a 2048-bit server certificate the code no longer
         appears, and `Server That Refuses Our Certificate Says So` passes under C.
      d) FOUND AFTER (a): **the C build could not offer `Basic128Rsa15` or `Basic256` at
         all.** open62541 compiles both into the library but leaves them out of the default
         policy set behind `UA_INCLUDE_INSECURE_POLICIES`, a macro with no CMake option. A
         device configured for either failed with a bare `BadInternalError` while Rust
         connected. Fixed by defining the macro for the open62541 targets in
         `impl/c/CMakeLists.txt`. `Basic256` then connects; `Basic128Rsa15` does not --
         open62541 aborts that handshake against UA-.NETStandard with `BadDecodingError`
         where async-opcua succeeds against the same endpoint. Recorded as the parity gap
         `opcua-basic128rsa15` (justfile capability lists, `impl/c/README.md`, `TODO.md`)
         rather than worked around: it is deprecated, opt-in only, and open62541 itself warns
         its encryption is broken.

## 2. Interop stack

- [x] 2.1 Establish the reference-server image. DONE: upstream publishes amd64 only and it
      aborts under qemu (.NET `AccessViolationException`), so we build upstream's unmodified
      `Dockerfile` from the pinned tag `v1.5.5` (`cef4b2494c125e03b4480d3eba33dfabf8c5ea00`)
      ourselves. Verified the arm64 image builds and serves 11 endpoints natively
- [x] 2.1b Add a `publish-refserver` job to `.github/workflows/publish-simulators.yaml`
      building that pinned ref for `linux/amd64,linux/arm64` and pushing
      `ghcr.io/thin-edge/tedge-dot/uanetstandard-refserver:v1.5.5`; verify the workflow is
      valid and the manifest list carries both architectures
- [x] 2.2 Write `connectors/opcua/interop/docker-compose.yaml` with the broker, the connector
      (honouring `IMPL`/`CONNECTOR_DOCKERFILE`) and the five server profiles `ref-all`,
      `ref-strict`, `ref-ecc`, `ref-token`, `ref-discovery` from our published image, with a
      `build:` fallback on the pinned git ref so a fresh checkout works before the image is
      published, and no fixed host ports; verify `docker compose config` is valid and every
      server reaches its healthcheck
- [x] 2.3 Add the namespace-resolution setup step: query the server's namespace array for
      `urn:opcua:testserver:nodes` using the `asyncua` client in the `opcua-sim` image and
      render `connector.toml` from a template; verify it prints the resolved index and that a
      deliberately wrong assumed index no longer appears anywhere in the rendered file
- [x] 2.4 Write the connector entrypoint for the stack: create the application certificate with
      `tedge-dot pki create`, export it into the volume `ref-strict` mounts at `/app/certs`,
      then start the connector; verify the exported certificate is present before `ref-strict`
      starts
- [x] 2.5 Add `just test-interop <proto>` and `just test-interop-c <proto>` mirroring `_e2e`
      (IMPL export, capability skips, own output directory); verify an unknown implementation
      argument is an error rather than an empty skip list
- [x] 2.6 Add a test proving the connector reaches a server that advertises
      `opc.tcp://0.0.0.0:<port>` as its endpoint URL (upstream hardcodes this and ignores
      `OPCUA_HOSTNAME`), where a client following the advertised URL gets
      `BadServerUriInvalid`; verify the device is `connected` and reads a value

## 3. Interop suite

- [x] 3.1 `connectors/opcua/interop/tests/` — policy and mode matrix against `ref-all`:
      `None`/`none`, `Basic256Sha256` with `sign` and `sign_and_encrypt`,
      `Aes128_Sha256_RsaOaep` and `Aes256_Sha256_RsaPss` with `sign_and_encrypt`. Verify each
      device is `connected`, publishes a sample, and reports the negotiated policy and mode in
      link-status `info`
- [x] 3.2 Deprecated policies on the wire: `Basic128Rsa15` and `Basic256` connect with
      `allow_deprecated_security = true`, and verify the same device without the opt-in fails
      configuration validation naming the device and the policy
- [x] 3.3 Identity coverage against `ref-all`: anonymous, username/password and X.509 accepted;
      wrong password gives `identity rejected:`; anonymous against a user-only endpoint gives
      `identity unsupported:`. Verify no password appears in the retained topics or the log
- [x] 3.4 Trust bootstrap: assert the first attempt quarantines the server certificate with
      `certificate untrusted:`, then `tedge-dot pki trust <thumbprint>` from
      `tedge-dot pki list rejected --json` brings the device to `connected` with no restart.
      Verify a second device with `trust_any_server_certificate = true` connects immediately
      and reports `server_certificate = not_verified`
- [x] 3.5 `ref-strict`: verify the device is `disconnected` with `application certificate
      rejected:` naming our own thumbprint, and that after the connector's certificate is in
      the server's trusted store and the service is restarted the device becomes `connected`
- [x] 3.6 Resource paths: verify a device against `ref-all`'s non-empty resource path and one
      against `ref-discovery`'s empty path both reach `connected`
- [x] 3.7 `ref-ecc`: verify the device is `disconnected` with `no matching endpoint:` listing
      the ECC policies offered, that another device keeps publishing, and that the connector
      neither exits nor busy-loops for the duration of the test
- [x] 3.8 `ref-token`: verify a device stays `connected` across at least two token lifetimes,
      samples keep arriving at the configured interval, and no reconnect is logged
- [x] 3.9 Address space: read the reference server's scalar variables for each datatype the
      connector claims, and subscribe to one of its timer-driven variables. Verify each sample
      carries the expected type and value and that subscription samples arrive without polling
      that point

## 4. CI and documentation

- [x] 4.1 Add an `interop` job to `.github/workflows/ci.yaml` on an amd64 runner running both
      implementations, independent of the `e2e` job so an upstream regression cannot block it;
      verify the job passes on a branch and that making it fail does not fail `e2e`
- [x] 4.2 Reuse the stack's registry-pull backoff for the pinned image the way
      `_pull-stack-images` does; verify a cold run does not fail setup on a rate-limited pull.
      DONE: `_interop` calls `_pull-stack-images` and `_prebuild-stack-images` exactly as
      `_e2e` does, so the image is pulled with backoff and, when it is not published yet,
      built from the pinned git context instead
- [x] 4.3 Document the interop layer in `doc/connectors/opcua-connector-spec.md` §10 and the
      new reason category in §5, and verify the reason prefixes listed there match the code
- [x] 4.4 Record the namespace-URI addressing gap in `TODO.md` as a follow-up, with the reason
      it matters (a server reordering its namespace array silently breaks an index-based
      configuration)
- [x] 4.5 Record the reason-category change for release notes. DONE by documentation: the
      repository has no CHANGELOG file (release notes come from conventional commits), so the
      change is written up in `doc/connectors/opcua-connector-spec.md` §5.3, which now states
      that a server-side rejection used to surface as `certificate untrusted:`. The commit
      message carries it for the generated notes

## 5. Verification

- [x] 5.1 Run `just test-interop opcua` and `just test-interop-c opcua` and verify both pass
      with no implementation-specific skips, EXCEPT the one genuine parity gap
      `requires:opcua-basic128rsa15`, which is skipped under C and runs under Rust. DONE:
      RUST 13/13 locally and on CI; C 13 passed / 0 failed / 1 skipped locally
- [x] 5.2 Run the existing suites — `just test`, `just conformance opcua`,
      `just conformance-c opcua`, `just test-e2e opcua`, `just test-e2e-c opcua` — and verify
      the connector fixes in section 1 broke nothing. DONE so far: `cargo test --workspace`
      (all green), `just conformance opcua` (147 passed, 0 failed), the `opcua-*` ctest
      targets and `just c-pki-parity`. `just conformance-c opcua`, `just test-e2e opcua` and
      `just test-e2e-c opcua` are covered by the CI jobs on PR #54. All 45 checks pass on
      f27826a, so the connector changes broke nothing
- [x] 5.3 Run `openspec validate opcua-reference-server-interop --strict` and verify every
      scenario in both spec deltas maps to a test that exists. Valid. The delta gained a
      MODIFIED `Explicit opt-out of server certificate validation` once the work showed the
      old wording ("any server certificate is accepted") was not achievable: a key outside the
      policy's range is refused whatever the trust settings say
