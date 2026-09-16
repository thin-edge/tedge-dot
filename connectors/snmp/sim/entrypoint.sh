#!/bin/sh
# Idle by default (the e2e suite sends every notification itself with `send-trap`). With
# TRAP_INTERVAL=<seconds> it sends a round of notifications periodically, for the demo.
#
# With FORWARD_TO=<host>:<port> it runs net-snmp snmptrapd as a trap forwarder instead: every
# notification received on udp/162 is forwarded to FORWARD_TO. `disableAuthorization yes` accepts
# any community; informs are acknowledged by snmptrapd itself. Observed with net-snmp 5.9.3: v1
# and v2c notifications are forwarded unchanged — no snmpTrapAddress.0 (1.3.6.1.6.3.18.1.3.0) is
# added (a v1 trap keeps the original sender in its agent-addr), and v3 notifications for users it
# does not know are not forwarded at all.
set -u

if [ -n "${FORWARD_TO:-}" ]; then
    mkdir -p /etc/snmp
    cat >/etc/snmp/snmptrapd.conf <<EOF
disableAuthorization yes
forward default udp:${FORWARD_TO}
EOF
    echo "SNMP trap forwarder: udp/162 -> ${FORWARD_TO}"
    # -f foreground, -Lo log to stdout, -n numeric addresses, -On numeric OIDs, -C only this
    # config file.
    exec snmptrapd -f -Lo -n -On -C -c /etc/snmp/snmptrapd.conf udp:162
fi

if [ "${TRAP_INTERVAL:-0}" -le 0 ]; then
    echo "SNMP simulator ready; send notifications with: send-trap <kind> (target ${TRAP_TARGET})"
    exec sleep infinity
fi

echo "SNMP simulator sending to ${TRAP_TARGET} every ${TRAP_INTERVAL}s"
n=0
while true; do
    n=$((n + 1))
    temp=$((800 + n % 100))
    send-trap overheat "Pump temperature high" "$temp" || true
    if [ $((n % 2)) -eq 0 ]; then send-trap linkdown 2 || true; else send-trap linkup 2 || true; fi
    [ $((n % 10)) -eq 1 ] && { send-trap coldstart || true; }
    sleep "$TRAP_INTERVAL"
done
