#!/bin/sh
# Portable post-remove script for deb/rpm/apk.
set -e

systemctl_available() {
    [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1
}

if systemctl_available; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi

tedge config remove c8y.smartrest.templates modbus || true
tedge refresh-bridges || true

# Purging (deb only; rpm and apk have no purge) also deletes the OPC UA PKI directory: the
# application certificate's private key and the trust lists. A plain removal keeps them, so a
# reinstall talks to the same servers with the same certificate.
if [ "${1:-}" = "purge" ]; then
    rm -rf /var/lib/tedge-dot/opcua
    rmdir /var/lib/tedge-dot 2>/dev/null || true
fi
