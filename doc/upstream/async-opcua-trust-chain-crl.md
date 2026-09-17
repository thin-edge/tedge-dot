# Upstream issue draft: async-opcua certificate store has no CA chain or CRL validation

Target: https://github.com/FreeOpcUa/async-opcua (crate `async-opcua-crypto` 0.18.0)

## Summary

`CertificateStore::validate_application_instance_cert`
(`async-opcua-crypto/src/certificate_store.rs`) trusts a peer certificate only if a
byte-identical copy exists in `<pki>/trusted/` under the name `cert_file_name()` produces.
The chain and revocation steps of OPC UA Part 4 §6.1.3 are placeholders:

```rust
// Other tests that we might do with trust lists
// ... issuer
// ... trust (self-signed, ca etc.)
// ... revocation
```

So a certificate issued by a site CA cannot be trusted by trusting the CA, and a revoked
certificate is never detected. Sites that run a PKI (the common case for OPC UA in plants)
have to copy every server or client certificate individually, and cannot revoke one.

Related:

1. `set_trust_unknown_certs(true)` writes every unknown certificate into `trusted/`, so the
   "accept anything" mode permanently trusts whatever was seen while it was on.
2. `ensure_pki_path` pushes and pops one path component per subdirectory, so a nested
   subdirectory name (as the Part 12 layout needs) creates the wrong tree.
3. `X509::create_from_pkey` uses serial number 42 for every generated certificate.
4. The layout (`own/cert.der`, `private/private.pem`, flat `trusted/` and `rejected/`) differs
   from OPC UA Part 12 Annex F.1 (`own/certs`, `own/private`, `trusted/certs`, `trusted/crl`,
   `issuers/certs`, `issuers/crl`, `rejected/certs`), which open62541 and the .NET stack use.

## Reproduction

1. Create a root CA and a server certificate it issued; place the CA certificate (only) in
   `<pki>/trusted/`.
2. Connect a client with `trust_server_certs(false)` to a server using that certificate on a
   `Basic256Sha256` endpoint.
3. Session creation fails with `BadCertificateUntrusted`, and the server certificate is copied
   into `<pki>/rejected/`.

## Suggested fix

What tedge-dot ships (`impl/rust/vendor/async-opcua-crypto`, see its `TEDGE-DOT-PATCH.md`,
item 2), offered as a starting point:

- Adopt the Part 12 layout (or make it configurable).
- Read trusted certificates, trusted CRLs, issuer certificates and issuer CRLs at validation
  time; recognise DER/PEM by content.
- Build the chain from the peer certificate through `issuers/` and `trusted/` CAs
  (basicConstraints cA, signature check) to a CA in `trusted/`; accept a pinned (identical)
  certificate directly.
- For every CA on the chain require a CRL signed by it (`Bad[Issuer]RevocationUnknown`
  otherwise, the Part 4 default) and check the subject's serial against it
  (`Bad[Issuer]Revoked`); check each CA's validity (`BadCertificateIssuerTimeInvalid`).
- Keep `trust_unknown_certs` as "accept without verification", without writing to `trusted/`.
- Use a random serial for generated certificates.

Test vectors covering pinned, CA-issued, intermediate, revoked, missing-CRL, foreign-CA,
issuer-only, expired, not-yet-valid, wrong-host, wrong-URI and short-key cases:
tedge-dot's `connectors/opcua/conformance/tools/genpki.py` (Python `cryptography`), with
expected outcomes in its generated `expected.json`.
