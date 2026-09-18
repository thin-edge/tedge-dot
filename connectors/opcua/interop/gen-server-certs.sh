#!/bin/sh
# Give the reference servers an RSA-2048 application certificate.
#
# WHY THIS EXISTS: UA-.NETStandard auto-generates a **1024-bit** RSA certificate
# (upstream says so in src/TestServer/Program.cs), which OPC UA Part 7 does not allow for
# Basic256Sha256, Aes128_Sha256_RsaOaep or Aes256_Sha256_RsaPss. A conformant client must
# refuse it -- open62541 does, at the security-policy layer, whatever the trust settings say.
# So without this the suite could not exercise a single modern policy against the reference
# server: every secured session would be refused for a reason that has nothing to do with
# interoperability.
#
# The server installs a pre-generated certificate only on the path it added for HTTPS
# (`OPCUA_ENABLE_HTTPS=true` -> InstallPregeneratedHttpsCertificate), which reads
# /app/certs/https-server/{cert,key}.pem and puts it in the store the opc.tcp endpoint
# presents from. That is why the servers that need a strong certificate also set
# OPCUA_ENABLE_HTTPS.
#
# UA-.NETStandard then validates what it was given, and rejects it unless:
#   - the subject CN equals the server's ApplicationName (OPCUA_SERVER_NAME),
#   - the URI SAN equals its ApplicationUri (urn:opcua:testserver:nodes),
#   - the SANs cover the base address, which it hardcodes to 0.0.0.0,
#   - it is an end-entity certificate (CA:FALSE, no keyCertSign).
# Each of those was found by watching it refuse a certificate that lacked it.
#
# ref-discovery is deliberately left with the weak auto-generated certificate: it is what
# `Weak Server Certificate Is Refused` needs.
set -eu

for name in "$@"; do
    dir="/out/$name/https-server"
    [ -f "$dir/cert.pem" ] && { echo "$name: certificate already present"; continue; }
    mkdir -p "$dir"

    cat > /tmp/ext.cnf <<EOF
[v3_req]
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment, dataEncipherment, nonRepudiation
extendedKeyUsage = serverAuth, clientAuth
subjectAltName = @alt_names

[alt_names]
URI.1 = urn:opcua:testserver:nodes
DNS.1 = $name
DNS.2 = localhost
IP.1 = 127.0.0.1
IP.2 = 0.0.0.0
EOF

    openssl genrsa -out "$dir/key.pem" 2048 2>/dev/null
    openssl req -new -x509 -days 3650 -key "$dir/key.pem" -out "$dir/cert.pem" \
        -subj "/O=tedge-dot interop/CN=$name" \
        -extensions v3_req -config /tmp/ext.cnf 2>/dev/null
    echo "$name: RSA-2048 certificate written to $dir"
done
