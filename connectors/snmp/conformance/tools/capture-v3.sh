#!/bin/sh
# Re-capture the SNMPv3 datagrams of v3captures.txt from net-snmp.
#
#   ./capture-v3.sh > v3captures.txt
#
# Needs net-snmp's snmptrap/snmpinform (5.6+) and python3. Every run produces different msgIDs,
# salts and digests — the datagrams are captured, not synthesised — so the committed file is
# what the vectors are generated from; re-run this only to refresh it (and then regenerate
# trap-vectors.json with genvectors.py).
#
# The credentials must stay in step with USERS in v3vectors.py.
set -eu

ENGINE=0x8000000001020304
USER=trapuser
AUTH=authpassword
PRIV=privpassword
TRAP="12345 1.3.6.1.6.3.1.1.5.3 1.3.6.1.2.1.2.2.1.1.3 i 3"

# Receive one datagram on a free UDP port while `$@` sends to it, and print it as hex.
capture() {
    python3 - "$@" <<'PY'
import os, socket, subprocess, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", 0))
s.settimeout(5)
argv = [a.replace("PORT", str(s.getsockname()[1])) for a in sys.argv[1:]]
sender = subprocess.Popen(argv, env=dict(os.environ, MIBS=""), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    print(s.recvfrom(65535)[0].hex())
finally:
    sender.kill()
PY
}

emit() { # emit <kind> <credential set> <name> -- <sender argv...>
    kind=$1 set=$2 name=$3
    shift 4
    printf '%s | %s | %s | %s\n' "$(capture "$@")" "$kind" "$set" "$name"
}

cat <<'HEADER'
# SNMPv3 datagrams captured from net-snmp for the `v3` section of trap-vectors.json
# (read by genvectors.py; regenerate with capture-v3.sh).
#
# Format: <hex datagram> | <kind> | <credential set> | <name>
HEADER

emit message sha-aes "net-snmp snmptrap -v 3 -l authPriv -a SHA -x AES: linkDown" -- \
    snmptrap -m '' -v 3 -e $ENGINE -u $USER -l authPriv -a SHA -A $AUTH -x AES -X $PRIV 127.0.0.1:PORT $TRAP
emit message sha "net-snmp snmptrap -v 3 -l authNoPriv -a SHA: linkDown" -- \
    snmptrap -m '' -v 3 -e $ENGINE -u $USER -l authNoPriv -a SHA -A $AUTH 127.0.0.1:PORT $TRAP
emit message md5-des "net-snmp snmptrap -v 3 -l authPriv -a MD5 -x DES: linkDown" -- \
    snmptrap -m '' -v 3 -e $ENGINE -u $USER -l authPriv -a MD5 -A $AUTH -x DES -X $PRIV 127.0.0.1:PORT $TRAP
emit rejected sha-aes "net-snmp snmptrap -v 3 -l authPriv with the WRONG authentication password" -- \
    snmptrap -m '' -v 3 -e $ENGINE -u $USER -l authPriv -a SHA -A wrongpassword -x AES -X $PRIV 127.0.0.1:PORT $TRAP
emit probe none "net-snmp snmpinform -v 3: the engine ID discovery probe" -- \
    snmpinform -m '' -r 0 -t 1 -v 3 -u $USER -l authPriv -a SHA -A $AUTH -x AES -X $PRIV 127.0.0.1:PORT $TRAP
