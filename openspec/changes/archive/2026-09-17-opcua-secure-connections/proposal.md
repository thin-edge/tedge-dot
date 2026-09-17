# Proposal

## Why

Real OPC UA servers in plants are rarely left on `SecurityPolicy#None`, yet neither connector
can connect to a secured server safely today. The Rust connector passes secured policies
through, but it trusts **every** server certificate (`trust_server_certs(true)`). It has no
client certificate configuration, and it writes a `./pki` directory into whatever working
directory it starts in. The C connector compiles open62541 with `UA_ENABLE_ENCRYPTION=OFF` and
rejects anything but `None` (parity gap `opcua-security`, "no test yet"). Secure connections
should be a supported, tested and documented feature in both implementations, with safe
defaults. They should not be a pass-through that happens to work.

## What Changes

- Secured channels in **both** implementations. Supported security policies are
  `Basic256Sha256`, `Aes128_Sha256_RsaOaep` and `Aes256_Sha256_RsaPss`, with modes `sign` and
  `sign_and_encrypt`. The deprecated `Basic128Rsa15` / `Basic256` policies are refused unless
  explicitly allowed. `None` stays available.
- **Application instance certificate**: the operator can configure a certificate and key, or
  the connector generates a self-signed one on first use. It is stored in a PKI directory and
  reused, and its URI SAN must match the `application_uri` setting.
- **PKI directory** with one layout shared by both implementations (`own/`, `trusted/`,
  `issuers/`, `rejected/`, each with `certs/` and CRLs where relevant). Its default is an
  absolute path. A relative path resolves against the config file, never the working
  directory.
- **Server certificate validation on by default**:
  - A certificate is trusted if it is pinned in `trusted/`, or if it chains to a CA in
    `trusted/` (with intermediates from `issuers/`) and passes the CRL checks.
  - The connector also checks the validity period, the host name, the application URI and the
    key length.
  - An untrusted certificate is copied to `rejected/`, and the device stays `disconnected`
    with a reason that names the certificate.
  - Trusting the certificate takes effect on the next reconnect, without a restart.
  - **BREAKING (Rust)**: the implicit trust-everything behaviour is removed. It can only be
    restored explicitly and per scope with `trust_any_server_certificate = true`, which logs a
    warning.
- **User identity tokens**: anonymous, username/password, and X.509 user certificates.
  - Passwords can be given inline or through `password_file`.
  - The connector refuses to send a password unencrypted unless this is explicitly allowed.
  - Secrets never appear in logs, in link status, or in `describe` output.
- **Endpoint selection**: the connector picks the server endpoint that matches the policy and
  mode, and it always dials the host and port from the config, even when the server
  advertises a different host name (NAT/gateway setups).
- **Diagnosable failures**:
  - The link-status `reason` states the category of each security failure: untrusted
    certificate, no matching endpoint, identity rejected, or a problem with the connector's own
    certificate.
  - The link-status `info` carries the effective policy and mode.
  - The connector warns ahead of its own certificate's expiry.
- **`tedge-dot pki` CLI** in both binaries: show or create the application certificate; list,
  trust, reject or remove server certificates; and add issuer certificates and CRLs.
- **C build**: open62541 is built with encryption (statically linked mbedTLS), so the
  `opcua-security` parity gap is removed.
- **Tests**:
  - The e2e stack gains a secured simulator endpoint.
  - The conformance suite gets a secured embedded server.
  - The parity harness runs the new suites against both implementations.
- Out of scope: GDS (push/pull certificate management), ECC policies, cloud-side certificate
  operations, and reverse connect.

## Capabilities

### New Capabilities
- `opcua-secure-connections`: how the OPC UA connectors set up secured sessions. This covers
  security policy and mode settings, the application instance certificate, server certificate
  trust (leaf and CA chain, CRLs), user identity tokens, endpoint selection, handling of
  secrets, and how security failures are reported.
- `opcua-pki-management`: the on-device PKI directory layout and the `tedge-dot pki` CLI that
  operators use to inspect and manage the application certificate, trusted and rejected server
  certificates, issuers and CRLs.

### Modified Capabilities
<!-- openspec/specs/ is empty; the OPC UA connector has no existing capability spec yet. -->

## Impact

- **Rust**:
  - `impl/rust/crates/connector-opcua` (`config.rs`, `connect_device`).
  - The vendored `impl/rust/vendor/async-opcua-crypto` gets a new patch that adds the shared
    PKI layout, CA-chain and CRL validation, and hooks for the rejected directory. Its
    `TEDGE-DOT-PATCH.md` needs updating.
  - `impl/rust/src/main.rs` gets the `pki` subcommand.
- **C**:
  - `impl/c/connectors/opcua/connector_opcua.c` (security config, trust list, identity).
  - `impl/c/CMakeLists.txt` switches to `UA_ENABLE_ENCRYPTION=MBEDTLS` and adds an mbedTLS
    source build (Apache-2.0).
  - `impl/c/cross/*`, because the glibc 2.17 floor must still hold.
  - `impl/c/src/main.c` gets the `pki` subcommand.
  - The parity table in `impl/c/README.md` changes.
- **Config and docs**:
  - `connectors/opcua/connector.toml`, `packaging/config/opcua.toml`, `demo/config/opcua.toml`.
  - A new `doc/connectors/opcua-connector-spec.md`, which is normative, as for SNMP.
  - The migration notes need a section for the breaking trust change.
- **Packaging**: the packages create the default PKI directory, owned by `tedge` with a private
  `own/private` directory.
- **Tests**:
  - `connectors/opcua/sim/server.py` (secured endpoint, certificates).
  - New e2e suite(s) under `connectors/opcua/tests/`.
  - `impl/rust/crates/ot-conformance` (secured OPC UA sim).
  - Unit and property tests for config validation, plus a fuzz target for certificate and CRL
    loading.
- **Dependencies**:
  - No new Rust crates beyond what `async-opcua-crypto` already pulls in (`rsa`, `x509-cert`).
    The `rsa` crate carries RUSTSEC-2023-0071, which the design addresses.
  - mbedTLS (Apache-2.0) is added for C.
