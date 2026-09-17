# Vendored `async-opcua-crypto` 0.18.0 with tedge-dot patches

Verbatim copy of the crates.io package, wired in through `[patch.crates-io]` in the workspace
`Cargo.toml`, with the changes below. Every patched spot carries a `tedge-dot patch` comment.
Drop this directory and the `[patch.crates-io]` entry once upstream covers all of them.

## 1. 32-byte session nonce on None-policy channels

`src/security_policy.rs`, `SecurityPolicy::random_nonce()`: `SecurityPolicy::None` returns 32
random bytes instead of a null ByteString.

Why: the async-opcua **server** uses it for the `serverNonce` of `ActivateSessionResponse`.
On a None-policy channel upstream sends an empty nonce; open62541 clients (1.4+) require a
session nonce of at least 32 bytes regardless of the security policy and fail the connection
with `BadSecurityChecksFailed` ("Session cannot be activated with a nonce that is too short").
Every other server stack we tested against (open62541 server, python-asyncua, commercial
servers) sends 32 bytes, so the conformance harness's embedded simulator would otherwise reject
the most common open-source client — including the C tedge-dot connector.
`src/tests/crypto.rs` (`generate_nonce`) asserts the patched length.

Upstream report draft: `doc/upstream/async-opcua-null-session-nonce.md`.

## 2. Trust lists with CA chains and CRLs, in the OPC UA Part 12 layout

Upstream trusts only a certificate whose exact copy sits in a flat `trusted/` directory under
an `<issuer> [<thumbprint>].der` name, has no chain or CRL handling (placeholder comments), and
with `trust_unknown_certs` *copies* every unknown certificate into `trusted/`. tedge-dot needs
the trust model of `doc/connectors/opcua-connector-spec.md` §4, identical to the C build's
(open62541):

- **`src/trust_list.rs` (new)**: `TrustList::load` reads `trusted/certs`, `trusted/crl`,
  `issuers/certs`, `issuers/crl` and `rejected/certs`, recognising DER or PEM by content and
  skipping unparsable files with a warning. `TrustList::verify` decides trust: rejected wins;
  a pinned (identical) certificate is trusted; otherwise the chain is built through
  `trusted/` and `issuers/` CAs (basicConstraints cA, RSA PKCS#1 v1.5 SHA-1/256/384/512 or
  RSASSA-PSS signature) to a CA in `trusted/`. Every CA on the path needs a CRL it signed
  (otherwise `BadCertificate[Issuer]RevocationUnknown`), revoked serials fail with
  `BadCertificate[Issuer]Revoked`, and a CA outside its validity period with
  `BadCertificateIssuerTimeInvalid`. Also `parse_certificates`, `parse_crls`,
  `crl_is_issued_by`, and the directory constants.
- **`src/certificate_store.rs`**:
  - `OWN_CERTIFICATE_PATH` / `OWN_PRIVATE_KEY_PATH` are `own/certs/cert.der` and
    `own/private/key.pem` (upstream: `own/cert.der`, `private/private.pem`), and exported.
  - `validate_application_instance_cert` reads the trust list on every call and uses
    `TrustList::verify`; an untrusted certificate is written to `rejected/certs`. With
    `trust_unknown_certs` every certificate is accepted and **nothing is written**. A key
    length that does not fit the policy is `BadCertificatePolicyCheckFailed` (upstream:
    `BadSecurityChecksFailed`). Time, host name and application URI checks are unchanged.
  - `cert_file_name` is `<CN>_<lower-case thumbprint>.der`.
  - `ensure_pki_path` creates the Part 12 layout (and joins each subdirectory to the PKI root:
    upstream's push/pop only worked for single-component names).
  - `new_with_x509_data` creates directories only when it may have to generate a certificate,
    so a None-policy client leaves the file system alone.
  - `read_cert` recognises DER or PEM by content, not by file extension.
  - Removed the now unused `store_trusted_cert` and `ensure_cert_and_file_are_the_same`.
- **`src/x509.rs`**: generated certificates get a random positive 128-bit serial number
  (upstream: always 42, so two generated certificates were indistinguishable by
  issuer + serial). New accessors `matches_private_key`, `alternate_names`, `issuer_name`,
  `is_ca`.
- **`src/x509.rs`**: `X509::from_byte_string` takes the first certificate of a DER chain
  (`from_der_first`); upstream requires exactly one certificate, so a server that sends its
  chain (OPC UA Part 6 §6.7.2) failed the connector's pre-check and the secure channel, and
  upstream's `create_session` silently skipped its host name / URI check. The chain's other
  certificates are ignored: CAs come from the trust list (`issuers/`). `from_der` stays strict.
- **`src/aes/rsa_private_key.rs`**: `PrivateKey::to_pem` (PKCS#8, LF line endings; upstream's
  store writes CR line endings).
- **`src/lib.rs`**: `pub mod trust_list`; re-exports the own-certificate path constants.
- **`src/tests/crypto.rs`**: `ensure_pki_path` checks the new layout;
  `certificate_chain_yields_the_leaf`.

The trust decisions are exercised by `impl/rust/crates/connector-opcua/tests/pki.rs` against
the vectors of `connectors/opcua/conformance/tools/genpki.py`, which the C build runs too.

Upstream report draft: `doc/upstream/async-opcua-trust-chain-crl.md`.

Run this crate's own tests with
`CARGO_TARGET_DIR=/tmp/crypto-target cargo test --manifest-path impl/rust/vendor/async-opcua-crypto/Cargo.toml`
(it is patched in, not a workspace member).
