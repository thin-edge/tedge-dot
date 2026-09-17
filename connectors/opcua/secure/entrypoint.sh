#!/bin/sh
# Entrypoint of the connector in the SECURED OPC UA stack (docker-compose.secure.yaml).
#
# The simulator generates the PKI vectors (genpki.py) into the shared /pki-vectors volume.
# This script waits for them, builds the connector's trust with `tedge-dot pki` -- the same
# CLI an operator uses -- and starts the connector on secure/connector.toml. The PKI
# directory is the packaged default, /var/lib/tedge-dot/opcua/pki.
#
# Trusted: the pinned, wrong-host and expired server certificates, and the root CA with its
# CRL (which revokes the "revoked" server). The intermediate CA is an issuer WITHOUT its CRL,
# so the server it issued fails with "revocation unknown" until the suite adds the CRL.
set -e
config=/opt/secure/connector.toml
v=/pki-vectors

echo "waiting for the PKI vectors ..."
until [ -f "$v/expected.json" ]; do sleep 1; done
for port in 4841 4842 4843 4844 4845 4846 4847 4848 4850; do
    echo "waiting for simulator-secure:$port ..."
    until nc -z simulator-secure "$port"; do sleep 1; done
done
echo "waiting for broker:1883 ..."
until nc -z broker 1883; do sleep 1; done

pki() { /usr/bin/tedge-dot pki "$@" --config "$config" >/dev/null; }
pki trust "$v/scenarios/pinned/server/cert.der"
pki trust "$v/scenarios/wrong_host/server/cert.der"
pki trust "$v/scenarios/expired/server/cert.der"
pki trust "$v/ca/root.der"
pki add-issuer "$v/ca/intermediate.der"
for crl in "$v"/scenarios/revoked/pki/trusted/crl/*; do pki add-crl "$crl"; done
# The suite adds this one itself (see "A Missing CRL Is Reported And Can Be Added").
cp "$v"/scenarios/intermediate/pki/issuers/crl/* /tmp/intermediate.crl

echo "starting tedge-dot (opcua, secured)"
exec /usr/bin/tedge-dot "$config"
