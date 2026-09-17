#!/usr/bin/env python3
"""Generate the OPC UA PKI test vectors shared by both implementations.

Every scenario is a complete tedge-dot PKI directory (the layout of
doc/connectors/opcua-connector-spec.md) plus the certificate and key a server presents,
and `expected.json` says what a client validating that server certificate against that
directory must conclude. The Rust unit tests, the C ctest suite, the conformance suite and
the e2e simulators all read the same tree, which is what makes the two trust validators
comparable (design D3).

Certificates are generated at test time, not committed: the expired / not-yet-valid cases
need dates relative to now, and committed private keys trip secret scanners. The structure
(scenario names, outcomes, subjects) is fixed; only keys, serials and dates change.

    genpki.py OUT_DIR [--hostname NAME ...] [--application-uri URI] [--client-uri URI]

Layout written:

    OUT_DIR/expected.json
    OUT_DIR/ca/root.der, root.key.pem, intermediate.der, intermediate.key.pem
    OUT_DIR/users/operator.der, operator.key.pem        X.509 user identity
    OUT_DIR/client/cert.der, key.pem                    a connector application certificate
    OUT_DIR/scenarios/<name>/pki/...                    the client's PKI directory
    OUT_DIR/scenarios/<name>/server/cert.der, key.pem   what the server presents

Outcomes map onto the link-status reason categories:
    trusted   -> the session is established
    untrusted -> "certificate untrusted:"
    invalid   -> "certificate invalid:"
    revoked   -> "certificate revoked:"   (also used for "revocation unknown")
"""

import argparse
import datetime
import ipaddress
import json
import os
import sys

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

DEFAULT_HOSTNAMES = ["localhost", "127.0.0.1"]
DEFAULT_SERVER_URI = "urn:tedge:opcua-sim"
DEFAULT_CLIENT_URI = "urn:tedge-dot"

NOW = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
DAY = datetime.timedelta(days=1)


def key(bits=2048):
    return rsa.generate_private_key(public_exponent=65537, key_size=bits)


def name(cn, org="tedge-dot test"):
    return x509.Name(
        [
            x509.NameAttribute(NameOID.COMMON_NAME, cn),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, org),
        ]
    )


def san(hostnames, uri):
    entries = []
    if uri:
        entries.append(x509.UniformResourceIdentifier(uri))
    for h in hostnames:
        try:
            entries.append(x509.IPAddress(ipaddress.ip_address(h)))
        except ValueError:
            entries.append(x509.DNSName(h))
    return x509.SubjectAlternativeName(entries)


def ca_cert(cn, k, issuer=None, issuer_key=None, path_len=None):
    """A CA certificate; self-signed when no issuer is given."""
    issuer_cert = issuer
    b = (
        x509.CertificateBuilder()
        .subject_name(name(cn))
        .issuer_name(issuer_cert.subject if issuer_cert else name(cn))
        .public_key(k.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(NOW - DAY)
        .not_valid_after(NOW + 3650 * DAY)
        .add_extension(x509.BasicConstraints(ca=True, path_length=path_len), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(k.public_key()), critical=False)
    )
    signer = issuer_key or k
    b = b.add_extension(
        x509.AuthorityKeyIdentifier.from_issuer_public_key(signer.public_key()), critical=False
    )
    return b.sign(signer, hashes.SHA256())


def app_cert(
    cn,
    k,
    hostnames,
    uri,
    issuer=None,
    issuer_key=None,
    not_before=None,
    not_after=None,
):
    """An OPC UA application instance certificate (Part 6 §6.2.2 profile).

    Self-signed when no issuer is given; a self-signed one carries keyCertSign as Part 6
    requires, so it can verify its own signature.
    """
    self_signed = issuer is None
    signer = issuer_key or k
    b = (
        x509.CertificateBuilder()
        .subject_name(name(cn))
        .issuer_name(issuer.subject if issuer else name(cn))
        .public_key(k.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(not_before or NOW - DAY)
        .not_valid_after(not_after or NOW + 365 * DAY)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=True,
                key_encipherment=True,
                data_encipherment=True,
                key_agreement=False,
                key_cert_sign=self_signed,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH, ExtendedKeyUsageOID.CLIENT_AUTH]),
            critical=False,
        )
        .add_extension(san(hostnames, uri), critical=False)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(k.public_key()), critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(signer.public_key()), critical=False
        )
    )
    return b.sign(signer, hashes.SHA256())


def user_cert(cn, k, issuer=None, issuer_key=None):
    """An X.509 user identity certificate (no application URI)."""
    signer = issuer_key or k
    b = (
        x509.CertificateBuilder()
        .subject_name(name(cn))
        .issuer_name(issuer.subject if issuer else name(cn))
        .public_key(k.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(NOW - DAY)
        .not_valid_after(NOW + 365 * DAY)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=True,
                key_encipherment=True,
                data_encipherment=True,
                key_agreement=False,
                key_cert_sign=issuer is None,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.CLIENT_AUTH]), critical=False
        )
    )
    return b.sign(signer, hashes.SHA256())


def crl(issuer, issuer_key, revoked_serials=()):
    b = (
        x509.CertificateRevocationListBuilder()
        .issuer_name(issuer.subject)
        .last_update(NOW - DAY)
        .next_update(NOW + 30 * DAY)
        .add_extension(x509.CRLNumber(1), critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(issuer_key.public_key()),
            critical=False,
        )
    )
    for s in revoked_serials:
        b = b.add_revoked_certificate(
            x509.RevokedCertificateBuilder()
            .serial_number(s)
            .revocation_date(NOW - DAY)
            .build()
        )
    return b.sign(issuer_key, hashes.SHA256())


def thumbprint(cert):
    return cert.fingerprint(hashes.SHA1()).hex()


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    os.chmod(path, mode)


def der(obj):
    return obj.public_bytes(serialization.Encoding.DER)


def pem(obj):
    return obj.public_bytes(serialization.Encoding.PEM)


def key_pem(k):
    return k.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )


def canonical(cert):
    """The file name tedge-dot writes certificates under: <CN>_<thumbprint>.der."""
    cn = cert.subject.get_attributes_for_oid(NameOID.COMMON_NAME)[0].value
    safe = "".join(c if c.isalnum() or c in "-." else "_" for c in cn)
    return f"{safe}_{thumbprint(cert)}.der"


def empty_pki(root):
    for d in (
        "own/certs",
        "own/private",
        "trusted/certs",
        "trusted/crl",
        "issuers/certs",
        "issuers/crl",
        "rejected/certs",
    ):
        os.makedirs(os.path.join(root, d), exist_ok=True)
    os.chmod(os.path.join(root, "own/private"), 0o700)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("--hostname", action="append", dest="hostnames",
                    help="SAN host names/IPs of the valid server certificates (repeatable; "
                         f"default {' '.join(DEFAULT_HOSTNAMES)})")
    ap.add_argument("--application-uri", default=DEFAULT_SERVER_URI,
                    help="the server's applicationUri (URI SAN of valid server certificates)")
    ap.add_argument("--client-uri", default=DEFAULT_CLIENT_URI,
                    help="the connector's application_uri (URI SAN of client/cert.der)")
    args = ap.parse_args()
    hosts = args.hostnames or DEFAULT_HOSTNAMES
    uri = args.application_uri
    out = args.out

    root_key = key()
    root = ca_cert("tedge-dot test root CA", root_key)
    inter_key = key()
    inter = ca_cert("tedge-dot test intermediate CA", inter_key, issuer=root, issuer_key=root_key, path_len=0)
    other_root_key = key()
    other_root = ca_cert("unrelated root CA", other_root_key)

    write(f"{out}/ca/root.der", der(root))
    write(f"{out}/ca/root.key.pem", key_pem(root_key), 0o600)
    write(f"{out}/ca/intermediate.der", der(inter))
    write(f"{out}/ca/intermediate.key.pem", key_pem(inter_key), 0o600)

    user_key = key()
    user = user_cert("operator", user_key)
    write(f"{out}/users/operator.der", der(user))
    write(f"{out}/users/operator.key.pem", key_pem(user_key), 0o600)

    client_key = key()
    client = app_cert("tedge-dot", client_key, ["localhost"], args.client_uri)
    write(f"{out}/client/cert.der", der(client))
    write(f"{out}/client/key.pem", key_pem(client_key), 0o600)

    scenarios = {}

    def scenario(sname, outcome, detail, server, server_key, trusted=(), trusted_crl=(),
                 issuers=(), issuers_crl=(), rejected=(), extra_trusted_files=None):
        base = f"{out}/scenarios/{sname}"
        pki = f"{base}/pki"
        empty_pki(pki)
        for c in trusted:
            write(f"{pki}/trusted/certs/{canonical(c)}", der(c))
        for c in trusted_crl:
            write(f"{pki}/trusted/crl/{thumbprint_crl(c)}.crl", c.public_bytes(serialization.Encoding.DER))
        for c in issuers:
            # PEM with an arbitrary name: file names and encodings must not matter.
            write(f"{pki}/issuers/certs/issuer-{len(os.listdir(pki + '/issuers/certs'))}.crt", pem(c))
        for c in issuers_crl:
            write(f"{pki}/issuers/crl/{thumbprint_crl(c)}.pem", c.public_bytes(serialization.Encoding.PEM))
        for c in rejected:
            write(f"{pki}/rejected/certs/{canonical(c)}", der(c))
        for fname, data in (extra_trusted_files or {}).items():
            write(f"{pki}/trusted/certs/{fname}", data)
        write(f"{base}/server/cert.der", der(server))
        write(f"{base}/server/cert.pem", pem(server))
        write(f"{base}/server/key.pem", key_pem(server_key), 0o600)
        scenarios[sname] = {
            "outcome": outcome,
            "detail": detail,
            "thumbprint": thumbprint(server),
        }

    # Pinned self-signed certificate.
    k = key()
    c = app_cert("sim pinned", k, hosts, uri)
    scenario("pinned", "trusted", "identical certificate in trusted/certs", c, k, trusted=[c])

    # Same, but trusted/certs also holds a file that is no certificate at all.
    k = key()
    c = app_cert("sim pinned corrupt", k, hosts, uri)
    scenario("corrupt_file", "trusted", "an unparsable file next to the pinned certificate is skipped",
             c, k, trusted=[c], extra_trusted_files={"garbage.der": b"\x30\x03not a certificate"})

    # Unknown self-signed certificate.
    k = key()
    c = app_cert("sim unknown", k, hosts, uri)
    scenario("untrusted", "untrusted", "neither pinned nor issued by a trusted CA", c, k)

    # Issued by a CA nobody trusts.
    k = key()
    c = app_cert("sim foreign ca", k, hosts, uri, issuer=other_root, issuer_key=other_root_key)
    scenario("foreign_ca", "untrusted", "issued by a CA that is not trusted", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key)])

    # rejected/certs is a list for review, not a deny list (Part 12): trust wins.
    k = key()
    c = app_cert("sim pinned and rejected", k, hosts, uri)
    scenario("rejected_listed", "trusted", "pinned wins over a copy in rejected/certs", c, k,
             trusted=[c], rejected=[c])

    # First seen (and quarantined) before its CA was trusted.
    k = key()
    c = app_cert("sim ca issued and rejected", k, hosts, uri, issuer=root, issuer_key=root_key)
    scenario("rejected_then_ca_trusted", "trusted",
             "a trusted chain wins over an automatic copy in rejected/certs", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key)], rejected=[c])

    # Only in rejected/certs.
    k = key()
    c = app_cert("sim only rejected", k, hosts, uri)
    scenario("rejected_only", "untrusted", "listed in rejected/certs and nowhere else", c, k,
             rejected=[c])

    # CA-issued, CA trusted with an empty CRL.
    k = key()
    c = app_cert("sim ca issued", k, hosts, uri, issuer=root, issuer_key=root_key)
    scenario("ca_issued", "trusted", "chains to a trusted CA with a CRL", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key)])

    # Through an intermediate kept in issuers/ (PEM, arbitrary name), CRLs for both.
    k = key()
    c = app_cert("sim intermediate issued", k, hosts, uri, issuer=inter, issuer_key=inter_key)
    scenario("intermediate", "trusted", "chains through issuers/ to a trusted root", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key)],
             issuers=[inter], issuers_crl=[crl(inter, inter_key)])

    # Intermediate without its CRL.
    k = key()
    c = app_cert("sim intermediate no crl", k, hosts, uri, issuer=inter, issuer_key=inter_key)
    scenario("intermediate_no_crl", "revoked", "revocation unknown: the intermediate CA has no CRL", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key)], issuers=[inter])

    # Intermediate in issuers/ but no trusted root: not trusted.
    k = key()
    c = app_cert("sim issuer only", k, hosts, uri, issuer=inter, issuer_key=inter_key)
    scenario("issuer_only", "untrusted", "issuers/ alone does not establish trust", c, k,
             issuers=[inter, root], issuers_crl=[crl(inter, inter_key), crl(root, root_key)])

    # Revoked by the CA.
    k = key()
    c = app_cert("sim revoked", k, hosts, uri, issuer=root, issuer_key=root_key)
    scenario("revoked", "revoked", "serial listed in the CA's CRL", c, k,
             trusted=[root], trusted_crl=[crl(root, root_key, [c.serial_number])])

    # CA without CRL.
    k = key()
    c = app_cert("sim ca no crl", k, hosts, uri, issuer=root, issuer_key=root_key)
    scenario("ca_no_crl", "revoked", "revocation unknown: the CA has no CRL", c, k, trusted=[root])

    # Pinned but expired / not yet valid.
    k = key()
    c = app_cert("sim expired", k, hosts, uri, not_before=NOW - 30 * DAY, not_after=NOW - DAY)
    scenario("expired", "invalid", "notAfter is in the past", c, k, trusted=[c])
    k = key()
    c = app_cert("sim not yet valid", k, hosts, uri, not_before=NOW + DAY, not_after=NOW + 30 * DAY)
    scenario("not_yet_valid", "invalid", "notBefore is in the future", c, k, trusted=[c])

    # Pinned but for another host / another application.
    k = key()
    c = app_cert("sim wrong host", k, ["plc.example.invalid"], uri)
    scenario("wrong_host", "invalid", "no SAN matches the endpoint host", c, k, trusted=[c])
    k = key()
    c = app_cert("sim wrong uri", k, hosts, "urn:somebody:else")
    scenario("wrong_uri", "invalid", "URI SAN differs from the server's applicationUri", c, k, trusted=[c])

    # Pinned but with a key too short for any supported policy.
    k = key(1024)
    c = app_cert("sim short key", k, hosts, uri)
    scenario("short_key", "invalid", "a 1024-bit key is below every supported policy's minimum", c, k,
             trusted=[c])

    expected = {
        "generated": NOW.isoformat(),
        "hostnames": hosts,
        "application_uri": uri,
        "client_uri": args.client_uri,
        "user_thumbprint": thumbprint(user),
        "client_thumbprint": thumbprint(client),
        "reason_prefix": {
            "untrusted": "certificate untrusted:",
            "invalid": "certificate invalid:",
            "revoked": "certificate revoked:",
        },
        "scenarios": scenarios,
    }
    write(f"{out}/expected.json", (json.dumps(expected, indent=2, sort_keys=True) + "\n").encode())
    return 0


def thumbprint_crl(c):
    h = hashes.Hash(hashes.SHA1())
    h.update(c.public_bytes(serialization.Encoding.DER))
    return h.finalize().hex()


if __name__ == "__main__":
    sys.exit(main())
