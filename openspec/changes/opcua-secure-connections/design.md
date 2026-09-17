# Design

## Context

See `proposal.md` for the motivation. The current state that shapes this design:

- **Rust** (`impl/rust/crates/connector-opcua/src/lib.rs`, `connect_device`):
  - The client is built with `trust_server_certs(true)` and `create_sample_keypair(false)`, and
    no certificate or key path is set. No application certificate ever exists, so a secured
    session cannot actually be established today. The breaking change therefore removes a
    trust default more than it breaks a working setup.
  - The PKI directory is async-opcua's default, `./pki` (relative to the working directory),
    which is why a stray `pki/` exists at the repository root.
  - `IdentityToken::UserName` is built from inline `user`/`password` values, but the endpoint
    tuple always passes `UserTokenPolicy::anonymous()`.
  - Unsecured devices dial the configured address directly. Secured devices use
    `connect_to_matching_endpoint`, which follows the host the server advertises.
- **Rust crypto**: async-opcua's `CertificateStore` (in the vendored
  `impl/rust/vendor/async-opcua-crypto`, patched once already) has these limits:
  - It trusts only exact leaf certificates, by a flat `trusted/<issuer>_<thumbprint>` file name
    plus a byte comparison.
  - It has no issuer or chain handling and no CRL check (the code has `// ... issuer`,
    `// ... revocation` placeholders).
  - It auto-creates `trusted/` and `rejected/` directories.
  - It uses pure-Rust `rsa` 0.9 and `x509-cert`.
- **C** (`impl/c/connectors/opcua/connector_opcua.c`):
  - open62541 v1.5.5 is built from source with `UA_ENABLE_ENCRYPTION OFF`.
  - `configure()` refuses any policy but `None`, and the connector always requests
    `SecurityPolicy#None`.
  - Parity gap `opcua-security` in `impl/c/README.md`.
  - The C release is zig-cross-compiled with a glibc 2.17 floor and avoids system shared
    libraries that suffer soname drift (the same reasoning rejected Debian's libsnmp).
- **Precedents**:
  - The SNMP spec established the `*_password_file` convention (read at configure time, first
    line) and the rule that secrets are never echoed.
  - The parity harness runs every suite with `IMPL=rust|c`.
  - The e2e stacks are DeviceLibrary compose projects with ephemeral ports.
  - Licences must be OSS-friendly (MPL-2.0 is the ceiling that has been accepted).

## Goals / Non-Goals

**Goals:**
- One PKI layout, one trust model and one set of failure categories, identical in Rust and C.
  Parity is proven by shared test vectors, not assumed.
- Configuration errors are caught at configure time. Runtime security failures are
  per-device, retried, and never take down other devices.
- Trust changes on disk take effect on the next connect without a reload.
- The C release keeps its glibc 2.17 floor and gains no dynamic crypto dependency.

**Non-Goals:**
- GDS push/pull, certificate-management server methods, and automatic certificate renewal.
- ECC security policies (`ECC_nistP256` and others). open62541 1.5.5 implements them with
  mbedTLS as well, but async-opcua does not, so they would be a parity gap from the start.
- Server-side security. Only the client connects; the conformance and e2e servers are test
  fixtures.
- Cloud-side trust or certificate operations, and reverse connect.
- HSM/TPM-held keys. Keys are files.

## Decisions

### D1 — tedge-dot owns the PKI layout; neither library's default is used
The layout in `specs/opcua-pki-management` follows OPC UA Part 12 (own / trusted / issuers /
rejected with `certs/` + `crl/`). async-opcua's layout (flat `trusted/`, `rejected/`) and
open62541's filestore layout differ from each other and from Part 12. Using either one would
make the directory implementation-specific, and the CLI and package would then differ by
build.

- *Alternative*: adopt open62541's filestore layout and teach Rust to read it. This was
  rejected because that layout is an implementation detail of open62541, and its
  auto-management (it writes files itself) conflicts with the CLI owning changes.

### D2 — Trust material is loaded by our code and re-read at every connect attempt
Both implementations read the `trusted/` and `issuers/` certificates and CRLs into memory when
a secured device connects. Each file is parsed as DER, or as PEM when it starts with
`-----BEGIN`. Unparsable files are skipped with a warning. The directories are small and
connects are rare (backoff), so re-reading is cheap, and it removes any need for file watching
or reloads.

- **Rust**: extend the vendored `CertificateStore` with a constructor that takes the in-memory
  trust list (trusted certificates, trusted CRLs, issuer certificates, issuer CRLs) and a
  `rejected/certs` path. Implement chain building, signature verification (RSA PKCS#1 v1.5
  and PSS with SHA-256 via the existing `rsa`/`sha2` dependencies), CRL signature and
  serial-number checks, and the "CA without CRL means revocation unknown" rule in
  `validate_application_instance_cert`, where the placeholders are.
  - Stop auto-creating library directories.
  - Record the patch in `TEDGE-DOT-PATCH.md`, and add an upstream draft in
    `doc/upstream/async-opcua-trust-chain-crl.md`.
  - *Alternative*: validate outside the library and set `trust_server_certs(true)`. This was
    rejected because the library would then accept whatever certificate the secure channel
    actually presents, and the check would not be bound to the channel.
- **C**: the verifier is implemented in our own code (`connectors/opcua/ua_pki.c`), not with
  open62541's in-memory certificate group. It is a line-by-line mirror of the Rust
  `TrustList::verify` on mbedTLS: chain building, `mbedtls_pk_verify_ext` signatures, CRL
  signature and serial checks. It is installed as a custom `UA_CertificateGroup` whose
  `verifyCertificate` validates the channel's server certificate and writes untrusted ones to
  `rejected/certs/`. With `trust_any_server_certificate`, `UA_CertificateGroup_AcceptAll` is
  installed instead.
  - *Why not the memory store (the original plan)*: task 3.1 showed that open62541 1.5.5's
    mbedTLS verifier differs from the spec in four ways:
    - It skips revocation entirely when no CRL is loaded, so a CA without a CRL is trusted.
    - It matches CRLs by issuer name only.
    - It reports a missing issuer as `ChainIncomplete`.
    - It does not accept a pinned CA-issued leaf.

    Post-checks around it would have been a second, harder-to-read verifier.
  - Confirmed against v1.5.5 (task 3.1): `UA_ClientConfig_setDefaultEncryption(config, cert,
    key, trustList, n, revocationList, n)` sets up the policies. Our group replaces the
    verification it installs. `UA_ClientConfig_setAuthenticationCert(config, cert, key)`
    handles X.509 user identities. `UA_CreateCertificate` is **not** used: it always writes
    serial number 1. `ua_pki_generate` builds the certificate with `mbedtls_x509write_crt`
    instead (random serial, URI SAN first).
  - The connector parses certificates, SANs and CRLs with mbedTLS directly. An installed
    open62541 is therefore used only if its `config.h` defines `UA_ENABLE_ENCRYPTION_MBEDTLS`
    and the mbedTLS headers and libraries are found. Otherwise the source build is used, and
    its link goes to the concrete `open62541` target, because the rejected package's imported
    `open62541::open62541` shadows the alias.
  - The source build switches off `UA_ENABLE_PUBSUB`, which needs method calls and fails to
    link in Debug builds, and `UA_ENABLE_DEBUG_SANITIZER`.
  - The discovery probe configures a `None` endpoint as its exact endpoint. Without that,
    open62541 first looks for an endpoint it could open a session on, which fails against
    servers that offer no `None` endpoint.

### D3 — Parity through shared PKI test vectors generated at test time
`connectors/opcua/conformance/tools/genpki.py` uses Python `cryptography` (Apache-2.0/BSD) and
writes a scenario tree. It covers these cases:
- a pinned self-signed certificate
- a CA-issued certificate with a CRL
- a revoked certificate
- a CA without a CRL
- an intermediate CA in `issuers/`
- an expired certificate
- a certificate that is not yet valid
- a wrong DNS SAN
- a wrong URI SAN
- a 1024-bit key
- a corrupt file

It also writes `expected.json` (outcome and reason category per scenario). The Rust unit
tests, the C ctest suite and the e2e simulator all consume these vectors. The certificates are
generated at test time, not committed, so that no fixture expires unnoticed. The script is
deterministic in structure, and only the dates and keys change.

- *Alternative*: commit fixtures with very long validity. This was rejected because the
  "expired" and "not yet valid" cases still need relative dates, and committed private keys
  trigger secret scanners.

### D4 — C crypto backend: mbedTLS 3.6 LTS, built from source and statically linked
`impl/c/CMakeLists.txt` gains a `FetchContent` for mbedTLS (Apache-2.0), with programs and
tests switched off, and sets `UA_ENABLE_ENCRYPTION=MBEDTLS`. open62541's mbedTLS plugin
supports every policy in scope and certificate creation (`UA_CreateCertificate`).

- *Alternative*: system OpenSSL. It is dynamic, has soname drift (1.1 vs 3), would break the
  glibc 2.17 packaging story and add a runtime dependency. This is the same reasoning that
  rejected Debian's libsnmp.
- *Alternative*: statically linked OpenSSL. It is large, and a heavier cross build under zig.
- A `find_package(MbedTLS)` preference is **not** added, for reproducible releases. An
  installed open62541 found by `find_package` must have encryption enabled; otherwise CMake
  fails with a clear message (or falls back to the source build).

### D5 — Rust crypto stays pure Rust; RUSTSEC-2023-0071 is accepted with justification
The `rsa` crate advisory (Marvin timing side channel) applies to RSA private-key operations.
The connector performs them only for the asymmetric OpenSecureChannel exchange and for signing
X.509 identity tokens. Those are a handful of operations per (re)connect, not an oracle an
attacker can query at will.

We document the exception in the audit configuration (`cargo deny`/`cargo audit` ignore entry,
with this rationale) and track upstream.

- *Alternative*: the `openssl` feature or crate. It brings back the system-OpenSSL
  cross-compile problem and the Darwin/zig issues the release already fought.

### D6 — Endpoint selection is done by the connector, then dialled at the configured address
For a secured device, the connector calls GetEndpoints on the configured URL, then selects the
endpoint whose policy URI and mode match. If several match, it takes the highest
`securityLevel`. The endpoint must also offer a user token policy for the configured identity
type. The connector then rewrites the selected endpoint's URL host and port to the configured
ones.

- **Rust**: pass the full `EndpointDescription` (including the server certificate) to
  `connect_to_endpoint_directly`. This replaces `connect_to_matching_endpoint`, so NAT setups
  work for secured devices too, and the correct `UserTokenPolicy` is passed instead of always
  `anonymous()`.
- **C**: call `UA_Client_getEndpoints`, then set `cc->endpoint`, `cc->userTokenPolicy`,
  `securityMode` and `securityPolicyUri`, and connect to the configured URL.

The "no matching endpoint" reason lists the pairs the server offered. Only anonymous
devices without message security keep today's direct path, with no GetEndpoints call. A
username or X.509 identity needs the endpoint's token policies (and, for token encryption, the
server certificate), so it discovers even on a `None` channel.

Before dialling, the connector validates the certificate the chosen endpoint advertises with
the same certificate store the session uses. This gives the exact reason category, thumbprint
and subject without opening a session, and it separates the two ways a secured connect can be
refused:
- **We do not trust the server:** the pre-check fails.
- **The server does not trust us:** the pre-check passes, but the session fails with
  `BadSecurityChecksFailed`/`BadCertificateUntrusted`. The reason then names this connector's
  own certificate thumbprint and `tedge-dot pki export`.

The session reports failures only as events, so the event loop records the last status code.
The connect attempt ends as soon as that code is a security failure, instead of waiting out
the library's retries.

### D7 — Identity and plaintext-password guard live in shared connector logic
- **Configuration**:
  - The identity fields are flat keys on `protocol_address`: `user`, `password`,
    `password_file`, `user_certificate` and `user_private_key`. These are backward-compatible
    with today's `user`/`password`.
  - `password_file` is read at configure time, following the SNMP spec. The Rust config type
    wraps the secret so that `Debug` prints `***`. The C code wipes the buffer on device
    teardown.
- **Plaintext guard**: before activation, the guard checks the selected token policy against
  Part 4 Table 193. The password is plaintext only on a mode-`None` channel whose username
  token policy names no policy (or `None`). If `allow_plaintext_password` is not set, the
  connector then fails with `plaintext password refused:`.
- **X.509 identity**: the user certificate and key are loaded at configure time. Rust uses
  `IdentityToken::X509`. C uses `UA_ClientConfig` `userIdentityToken` with
  `UA_X509IdentityToken` and sets `authenticationSecurityPolicyUri` so the token is signed.

### D8 — Application certificate lifecycle
- **Loading**: the certificate is loaded (or generated) at configure time, only if a secured
  device exists. If it cannot be obtained, the error is reported per device (secured devices
  `disconnected`, reason `application certificate: …`); unsecured devices still start. This
  mirrors the SNMP "one bad device must not take the gateway down" rule.
- **Generation**:
  - Rust uses `X509::cert_and_pkey` from the vendored crate. C uses `UA_CreateCertificate`
    (mbedTLS).
  - Both use a 2048-bit RSA key, SHA-256, a URI SAN set to `application_uri`, and DNS SANs set
    to the host name plus any CLI `--hostname` values. Validity is 5 years.
  - Files are written to a temp file and then renamed, with the key created at 0600.
  - `own/.lock` is held with `flock` around the check-and-generate step, so concurrent
    connectors (several `opcua*.toml` files in one `tedge-dot run`, or Rust and C side by side)
    generate only once.
- **Expiry and renewal**:
  - A daily timer warns of expiry within 30 days, and an expired certificate becomes a
    per-device failure.
  - A renewed certificate (`pki create --force`) takes effect on reload (SIGHUP) or restart.
    This is documented, and the spec does not promise hot-swapping the own certificate.
- **Validation**: a URI SAN mismatch with `application_uri` is detected at load, because many
  servers reject it with an unhelpful `BadCertificateUriInvalid`.

### D9 — `tedge-dot pki` is a thin, offline CLI in both binaries
- **Rust**: a `Pki(PkiArgs)` variant in `impl/rust/src/main.rs`. The logic lives in
  `connector-opcua` (`src/pki.rs`, sharing the loader from D2) behind the existing `opcua`
  feature.
- **C**: `impl/c/connectors/opcua/pki_cli.c`, dispatched from `main.c`'s `is_subcommand`.
- **Behaviour**: config resolution follows the spec. Output is a plain table, or JSON with
  `--json`, and the JSON field names are the same in both implementations (checked by a
  parity test like `describe-parity.sh`).
- **File handling**:
  - Moves are `rename(2)` within the PKI directory.
  - Imports parse the file first and write it DER-normalised under the canonical name.
  - When run as root on a PKI directory another user owns, the PKI work runs with that user's
    effective user and group (`seteuid`), so new files are the owner's and symlinks the owner
    placed cannot redirect root's writes; only the file an action reads and the
    `export --output` file are accessed as root. (Review follow-up; replaced a `chown` pass.)

### D10 — Link status and reasons
The `LinkStatus.info` object (currently `None` for OPC UA) carries these fields:
`{ endpoint, security_policy, security_mode, server_certificate?, server_thumbprint? }`.

Reason strings are built from a fixed set of category prefixes (see the spec), followed by
free text. Tests assert only the prefix, and the thumbprint for `certificate untrusted:`.
Library status codes are mapped to categories:

| Status code | Category |
| --- | --- |
| `BadCertificateUntrusted`, `BadSecurityChecksFailed` during validation | `certificate untrusted:` |
| `BadCertificateTimeInvalid`, `BadCertificateHostNameInvalid`, `BadCertificateUriInvalid`, `BadCertificateInvalid`, `BadCertificateIssuerTimeInvalid`, `BadCertificatePolicyCheckFailed` (key too short for the policy), `BadCertificateUseNotAllowed`, `BadCertificateChainIncomplete` | `certificate invalid:` |
| `BadCertificateRevoked`, `BadCertificateIssuerRevoked`, `BadCertificateRevocationUnknown`, `BadCertificateIssuerRevocationUnknown` | `certificate revoked:` |
| `BadIdentityTokenRejected`, `BadIdentityTokenInvalid`, `BadUserAccessDenied` | `identity rejected:` |

A revocation-unknown failure uses the `certificate revoked:` prefix, and its text says
"revocation unknown".

### D11 — Test layers
- **Unit and property tests** (both implementations):
  - config validation (policy/mode matrix, identity combinations, deprecated opt-in, relative
    `pki_dir`)
  - the D3 vector scenarios against the trust loader and validator
  - a proptest that no validation error string ever contains the password
- **Fuzzing**: a Rust fuzz target for the PEM/DER certificate and CRL loader. A C libFuzzer
  target is optional and is added if the C fuzz setup exists by then.
- **Conformance**: the ot-conformance embedded async-opcua server gains a secured endpoint
  (`Basic256Sha256`, username and X.509 users) with its certificate from `genpki.py`.
  `opcua_conformance.rs` runs against both implementations.
- **e2e**: `server.py` (asyncua 1.1) gains env-driven security (policies, server
  certificate/key paths, a user database, client certificate trust). The compose stack adds
  these services from the same image:
  - `simulator-secure`: a CA-issued certificate, with the correct SAN.
  - `simulator-selfsigned`: a pinned-certificate scenario.
  - `simulator-badcert`: a wrong SAN, and an expired certificate on a second port.

  A new `connectors/opcua/tests/opcua_security_e2e.robot` covers these flows:
  - quarantine → `tedge-dot pki trust` → connected
  - CA trust
  - revoked certificate and missing CRL
  - host name mismatch
  - username via `password_file`
  - X.509 user identity
  - plaintext refusal
  - `trust_any_server_certificate`
  - no matching endpoint
  - secrets absent from the retained topics

  Stack names stay ephemeral (no fixed host ports). The PKI trees are generated in suite setup
  and mounted into the connector and simulators.

## Risks / Trade-offs

- **[Risk] Our chain and CRL validation in the vendored Rust crate is security-critical code
  we now own.** → Keep it small and use only existing primitives. Every rejection path gets a
  D3 vector, and the same vectors run against open62541 as an independent oracle. The code is
  also fuzzed, and the patch is offered upstream.
- **[Risk] open62541 and our Rust code disagree on edge semantics** (missing CRL, a CA
  certificate also pinned as a leaf, an expired intermediate). → The D3 vectors define the
  expected outcome. Where open62541 differs, the C wrapper (D2) post-checks and adjusts, and
  the difference is documented in the connector spec.
- **[Risk] mbedTLS under zig cross builds or the glibc 2.17 floor fails, or the binary grows
  too much.** → Verify early (task group 3, before the connector work). `cross/verify.sh`
  asserts the maximum GLIBC symbol version. Record the size increase in the PR; roughly
  +0.5 MB is expected.
- **[Risk] RUSTSEC-2023-0071 is flagged by audits.** → Documented exception (D5). Revisit when
  `rsa` 0.10 ships a constant-time fix.
- **[Trade-off] "CA without CRL = rejected" is strict** and will surprise sites with no CRL
  process. → The reason text says to add an (empty) CRL. The connector spec and the CLI
  `list` output flag CAs without CRLs. `trust_any_server_certificate` remains the explicit
  escape hatch.
- **[Risk] Clock skew on RTC-less gateways makes valid certificates look expired or not yet
  valid.** → The reason names the certificate dates and the local time. No time-check bypass
  is added.
- **[Risk] A private key readable by other users.** → The loader refuses a key file whose mode
  grants group or other access (warning plus failure), and generation creates it with 0600.
- **[Trade-off] A renewed own certificate needs a reload.** → Documented, and the CLI prints a
  reminder after `create --force`.

## Migration Plan

1. `None`/`none` configurations need no action, and their behaviour is unchanged (no
   GetEndpoints call, no PKI directory).
2. Rust configurations that set a secured policy never had an application certificate, so they
   could not have worked. After the upgrade, they generate one on first start and quarantine
   the server certificate. The operator then runs `tedge-dot pki list rejected` and
   `tedge-dot pki trust <thumbprint>`, and hands `tedge-dot pki export` to the server admin.
   This goes into `doc/migration/migration-guide.md` and the release notes as **BREAKING**
   (removal of implicit trust).
3. The packages create `/var/lib/tedge-dot/opcua/pki` in postinst. Upgrades leave it alone.
   `purge` removes it.
4. The stray repository-root `pki/` directory is deleted and added to `.gitignore`, since
   tests now use temporary PKI directories.
5. **Rollback**: downgrading the package leaves the PKI directory unused but in place. The old
   Rust build falls back to `./pki`, and the old C build refuses secured policies, as before.
