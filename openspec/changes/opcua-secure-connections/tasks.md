# Tasks

## 1. Normative spec and test vectors

- [x] 1.1 Write `doc/connectors/opcua-connector-spec.md` from `_template-connector-spec.md`. It must cover the `connection` and `protocol_address` security fields, the PKI layout, the trust rules, identity, the reason categories, the link-status `info` and `tedge-dot pki`. Link it from `doc/connectors/` and `doc/README.md`, and verify that the markdown renders and the wordlist spell check passes.
  - The repository has no spell-check tooling (`wordlist.txt` is git-ignored, and cSpell runs only as an editor extension), so that part of the check does not apply. The spec is linked from `README.md`, `doc/README.md` and the template.
- [x] 1.2 Add `connectors/opcua/conformance/tools/genpki.py` (Python `cryptography`), which generates the D3 scenario tree (pinned, CA+CRL, revoked, CA without CRL, intermediate in `issuers/`, expired, not-yet-valid, wrong DNS SAN, wrong URI SAN, 1024-bit, corrupt file, username/X.509 user certificates) plus `expected.json`. Verify it by running the script twice into temporary directories: the scenario names and expected outcomes must match, and `openssl verify` must agree for the chain cases.
- [x] 1.3 Add `cryptography` to `requirements-test.txt` and the simulator requirements. Delete the stray repository-root `pki/`, add it to `.gitignore`, and verify that `git status` is clean after a full Rust test run.

## 2. Rust: configuration and validation

- [x] 2.1 Extend `OpcuaConnection` and `OpcuaEndpoint` in `connector-opcua/src/config.rs` with these fields: `pki_dir`, `certificate`, `private_key`, `create_certificate`, `trust_any_server_certificate`, `allow_deprecated_security`, `allow_plaintext_password`, `password_file`, `user_certificate` and `user_private_key`. Add a secret wrapper whose `Debug` output is `***`. Verify with unit tests that parse each field.
- [x] 2.2 Implement configure-time validation: the policy and mode matrix, the deprecated-policy opt-in, identity exclusivity, `password_file` reading (first line, path-only errors), resolution of a relative `pki_dir` against the config file directory, and private key permission checks. Verify with unit tests for every spec validation scenario.
- [x] 2.3 Add a proptest in `connector-opcua` asserting that no validation error or `Debug` output contains the password or the password file contents, and verify that `cargo test -p connector-opcua` passes.

## 3. C build: encryption enabled (de-risk first)

- [x] 3.1 Add a mbedTLS 3.6 LTS `FetchContent` build (programs and tests off) to `impl/c/CMakeLists.txt` and set `UA_ENABLE_ENCRYPTION=MBEDTLS`. Confirm the open62541 1.5.5 API names used in D2, D4 and D7 (`UA_CertificateGroup_Memorystore`, `UA_TrustListDataType`, `UA_CreateCertificate`, the X.509 identity token fields) and note any deviation in `design.md`. Verify that a native `cmake --build` and the existing ctest suite pass.
- [x] 3.2 Make an installed open62541 without encryption fail the CMake configure step with a clear message, or fall back to the source build. Verify by configuring against a stub package.
- [x] 3.3 Update `impl/c/cross/Dockerfile`, `build.sh` and `verify.sh` for the mbedTLS build. Verify that `just c-cross` produces x86_64 and arm64 binaries whose highest GLIBC symbol is 2.17, and record the change in binary size in the PR description.
  - Result: arm64 1,111,784 bytes (main: 720,864, so +391 KB) with max GLIBC 2.17. amd64 with `-DTDOT_SNMP=OFF` is 791,584 bytes with max GLIBC 2.17. A full amd64 cross build from an arm64 host fails in net-snmp's bundled MD5 (x86 inline asm rejected by zig's clang); `main` fails the same way, so this is not caused by this change. `build.sh` now enforces the glibc floor.

## 4. Rust: PKI store and trust validation (vendored async-opcua-crypto patch)

- [x] 4.1 Implement the shared PKI loader in `connector-opcua/src/pki.rs`. It reads the layout from the spec, parses DER or PEM by content, skips corrupt files with a warning, writes files atomically under canonical names, and holds an `own/.lock` flock around certificate generation. Verify with unit tests, including the corrupt-file and concurrent-generation scenarios, using two threads or processes.
- [x] 4.2 Patch `impl/rust/vendor/async-opcua-crypto/src/certificate_store.rs` to add an in-memory trust-list constructor, chain building with issuers, RSA signature verification (PKCS#1 v1.5 and PSS, SHA-256), CRL signature and serial checks, the rule that a CA without a CRL means revocation unknown, `rejected/` as a review list that does not override trust (review follow-up), writing to `rejected/certs/`, and no directory auto-creation. Keep the time, hostname, URI and key-length checks. Verify with the crate's own tests plus new tests driven by the `genpki.py` vectors, all of which must match `expected.json`.
- [x] 4.3 Update `TEDGE-DOT-PATCH.md` and add `doc/upstream/async-opcua-trust-chain-crl.md` (an upstream report draft). Verify that both describe every patched function.
- [x] 4.4 Add a fuzz target for the PEM/DER certificate and CRL loader (a new `connector-opcua/fuzz` crate, or an addition to the existing fuzz workspace), seeded with the `genpki.py` output. Verify that it runs for 60 seconds with no crash and is listed in `doc/testing.md`.
- [x] 4.5 Add the RUSTSEC-2023-0071 exception with the D5 rationale to the audit configuration (create `deny.toml` or `.cargo/audit.toml` if absent), and verify that `cargo audit` or `cargo deny check advisories` passes.
  - `impl/rust/.cargo/audit.toml` now ignores RUSTSEC-2023-0071 only. The two other advisories that predated this change (`h2` RUSTSEC-2026-0258, `rustls` RUSTSEC-2026-0285) are fixed by updating the lockfile: `h2` 0.4.19, and `rustls` 0.23.45, which also moves `aws-lc-rs`/`aws-lc-sys`. Both crates reach only the `ot-conformance` test harness (through `reqwest`/`jsonschema`), not the shipped binary. `cargo audit` now reports no vulnerabilities.

## 5. Rust: secured connect path

- [x] 5.1 Load or generate the application certificate at configure time, only when a secured device exists. Check the URI SAN against `application_uri`. Report failures per device as `application certificate:`, while unsecured devices still start. Verify with integration tests covering generation, reuse across restarts, URI mismatch and the unsecured-only case (no directory created).
- [x] 5.2 Replace `trust_server_certs(true)` and `connect_to_matching_endpoint` in `connect_device`: call GetEndpoints, select the endpoint by policy, mode, token type and highest `securityLevel`, rewrite the host and port to the configured values, rebuild the trust list per attempt, and call `connect_to_endpoint_directly(EndpointDescription, identity)`. Verify with an integration test against an in-process secured async-opcua server that advertises an unreachable host name.
- [x] 5.3 Implement the identity tokens (anonymous, username with a password or `password_file`, X.509), the "identity unsupported" check against the selected endpoint, and the plaintext-password guard with `allow_plaintext_password`. Verify with integration tests for each identity scenario in the spec.
- [x] 5.4 Implement `trust_any_server_certificate` (warning on each connect, `info.server_certificate = "not_verified"`) and the categorised reasons (D10 mapping, thumbprint and subject for untrusted certificates). Fill `LinkStatus.info` with the endpoint, policy, mode, server certificate state and thumbprint. Verify with integration tests asserting the reason prefixes and the `info` fields.
- [x] 5.5 Add a daily expiry warning for the own certificate (within 30 days), and have an expired certificate disconnect secured devices. Verify with a unit test that uses a short-lived generated certificate and an injectable clock.

## 6. C: secured connect path

- [x] 6.1 Extend `configure()` in `connector_opcua.c` with the same fields and validation as 2.1 and 2.2 (including `password_file`, relative `pki_dir` and key permissions), replacing the "None only" refusal. Verify with new cases in `impl/c/tests/config.c`.
- [x] 6.2 Implement the PKI loader and atomic writer with `flock` in `impl/c/connectors/opcua/pki.c`, and application certificate load or generation via `UA_CreateCertificate`. Verify with ctest cases for corrupt files, reuse and URI mismatch.
- [x] 6.3 Build a `UA_TrustListDataType` per connect attempt, install the in-memory certificate group, and wrap `verifyCertificate` to write to `rejected/certs/` and record the status code (D2). Verify with a ctest that runs the `genpki.py` vectors through the verifier and matches `expected.json`, adding post-checks wherever open62541 disagrees.
  - As built: a custom `UA_CertificateGroup` backed by `ua_pki.c` (a mirror of the Rust verifier) instead of the memory store plus post-checks. See design.md D2 for why.
- [x] 6.4 Implement endpoint selection with `UA_Client_getEndpoints`, the configured-host dialling, the identity tokens (username, X.509 with `authenticationSecurityPolicyUri`), the plaintext guard, `trust_any_server_certificate`, the reason categories and the link-status `info`. Wipe secrets on teardown. Verify with the conformance suite in 8.2 under `IMPL=c`.
- [x] 6.5 Add the own-certificate expiry warning and expired-certificate handling as in 5.5, and verify with a ctest that uses a short-lived certificate.
- [x] 6.6 Remove the `opcua-security` row from the parity table in `impl/c/README.md` (or mark it ✅/✅), and verify that the parity harness fallback guard reports no OPC UA security skip.

## 7. `tedge-dot pki` CLI (both binaries)

- [x] 7.1 Rust: add `Pki(PkiArgs)` to `impl/rust/src/main.rs` with the actions `show`, `export`, `create`, `list`, `trust`, `reject`, `remove`, `add-issuer` and `add-crl`, config resolution (`--pki-dir`, then `--config`, then the default config, then the default directory), `--json`, exit codes 0/1/2, thumbprint prefix matching (at least 8 hex digits, ambiguity error), and chown to the directory owner when running as root. Verify with CLI tests using `assert_cmd` or the existing test pattern, covering every scenario in the `opcua-pki-management` spec.
- [x] 7.2 C: add `impl/c/connectors/opcua/pki_cli.c` and dispatch it from `main.c` (`is_subcommand` and usage), with the same actions, options, exit codes and JSON field names. Verify with ctest cases that mirror 7.1.
- [x] 7.3 Add `impl/c/ci/pki-parity.sh` (modelled on `describe-parity.sh`), which runs the same `pki` command sequence against both binaries on a `genpki.py` tree and diffs the normalised JSON. Wire it into CI and verify that it passes.

## 8. Simulators, conformance and e2e

- [x] 8.1 Extend `connectors/opcua/sim/server.py` with environment-driven security: the policy list, server certificate and key paths, users (username and password), and a trusted client certificate directory for X.509 users. The existing unsecured default must be unchanged. Verify that the existing `opcua_e2e.robot` still passes with `IMPL=rust` and `IMPL=c`.
- [x] 8.2 Add a secured endpoint to the ot-conformance embedded server (`src/sim/opcua.rs`): `Basic256Sha256` and `Aes256_Sha256_RsaPss`, username and X.509 users, and a certificate from `genpki.py`. Add secured cases to `connectors/opcua/conformance*.toml`, and verify that `just conformance opcua` and `just conformance-c opcua` both pass.
- [x] 8.3 Add the `simulator-secure` (CA-issued), `simulator-selfsigned` and `simulator-badcert` services to `connectors/opcua/docker-compose.yaml`, with ephemeral ports and PKI trees generated in suite setup and mounted read-only into the simulators and read-write into the connector. Verify that DeviceLibrary strict validation accepts the stack.
- [x] 8.4 Write `connectors/opcua/tests/opcua_security_e2e.robot` covering quarantine → `tedge-dot pki trust` → connected without restart, CA trust, a revoked certificate, a missing CRL, a host name mismatch, an expired certificate, username via `password_file`, an X.509 user, plaintext refusal, `trust_any_server_certificate`, no matching endpoint, and a check that no retained topic contains secrets. Verify that it passes with `IMPL=rust` and `IMPL=c` (`just test-e2e opcua`, `just test-e2e-c opcua`).

## 9. Packaging, configs and docs

- [x] 9.1 Add the creation of `/var/lib/tedge-dot/opcua/pki` (owner `tedge`, mode 0750, `own/private` at 0700) to the Rust and C nfpm and postinst scripts, with removal only on purge. Verify with the release smoke test (install, upgrade, then check the directory's ownership, modes and contents).
  - Verified by hand with an nfpm-built arm64 `.deb` in `debian:bookworm-slim`: the directories are created with the right owners and modes and no files; `tedge-dot pki create`, run as root, hands its files to `tedge`; a reinstall and `dpkg -r` keep the certificate; `dpkg --purge` removes `/var/lib/tedge-dot`. The release workflow's smoke test was not changed.
- [x] 9.2 Document the security options (commented out, secure-by-example) in `packaging/config/opcua.toml` and `demo/config/opcua.toml`, and add an optional secured device to the demo compose file. Verify that the demo still starts with the defaults, and that `tedge-dot describe` output contains no secrets.
- [x] 9.3 Add a **BREAKING** section to `doc/migration/migration-guide.md` about removing implicit server trust in Rust, with the `tedge-dot pki` workflow for first contact and a note on the CRL requirement for CA trust. Update the OPC UA sketch in `_template-connector-spec.md` to point to the new spec, and verify that the docs build and spell check pass.
- [x] 9.4 Final check: run `cargo test --workspace`, the C ctest suite, `just c-cross`, both conformance runs, both e2e suites and the parity scripts, and confirm that all of them are green before running `/opsx:archive`.
  - Results (macOS arm64 host):
    - `cargo test --workspace` and clippy: green.
    - ctest: 6/6.
    - Conformance: Rust 129 + 147 (secured), C 128 + 146 (secured).
    - e2e (`opcua_e2e` + `opcua_security_e2e`): 46/46 for both Rust and C.
    - `pki-parity.sh` and `describe-parity.sh`: pass.
    - `just c-cross arm64`: GLIBC 2.17.
    - The full amd64 cross build fails in net-snmp on an arm64 host, as it does on `main` (see 3.3).

## 10. Review follow-ups

- [x] 10.1 Refuse management commands that add or change local-only settings (`Connector::local_only_settings` / `tdot_connector.local_only_settings`, checked by the runtime next to the path-reference guard); OPC UA and SNMP declare theirs. Verified by `reject_local_only_settings` unit tests (Rust) and `check_local_only_settings` (C).
- [x] 10.2 `rejected/certs` is a review list, not a deny list: trust (pinned or chained) wins over a copy there; `pki reject` warns when a CA still vouches for the certificate. Vectors `rejected_listed` (now trusted), `rejected_then_ca_trusted`, `rejected_only`.
- [x] 10.3 `pki remove`/`reject`/`trust` take one certificate out of a PEM bundle instead of deleting the file (both builds; a duplicate within one file is handled once).
- [x] 10.4 C `pki` CLI: no `system("find … chown")` (superseded by 11.1).
- [x] 10.5 Rust: a server certificate chain is accepted (the leaf is validated), via `X509::from_byte_string` in the vendored crypto crate, which also covers the secure channel and upstream's `create_session` check.
- [x] 10.6 A private key without a certificate (interrupted generation) is set aside and a new certificate generated; a certificate without its key is an explicit error (both builds).
- [x] 10.7 C: the password read for the type check is wiped and freed.
- [x] 10.8 Endpoint selection requires a listed token policy for anonymous identities too (both builds agree; neither client library can activate without one).

## 11. Second review follow-ups

- [x] 11.1 `tedge-dot pki` run as root on a PKI directory another user owns acts as that owner (effective user and groups, both builds); the admin-named input and `export --output` files are accessed as root. The chown pass is gone. `write_atomic` creates its temporary file with `O_EXCL | O_NOFOLLOW` and sets the mode on the descriptor. Verified by the root-only part of `opcua_pki_cli.sh` in the Linux images.
- [x] 11.2 A password on a channel without message security is unprotected even with token encryption (no server certificate is authenticated there; async-opcua encrypts to the unvalidated CreateSession certificate), and so is one on a signed-only channel whose token policy is `None`; both need `allow_plaintext_password`.
- [x] 11.3 Removing a local-only key from a device that remains, or from `[connection]`, is refused like adding or changing one.
- [x] 11.4 C accepts a DER certificate or CRL only when it fills the whole buffer (as Rust does); `--days` is 1–100000 in both builds. Both in `pki-parity.sh`.
