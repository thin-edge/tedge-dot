#!/usr/bin/env bash
# End-to-end smoke test for the C connector against a protocol simulator
# and a live MQTT broker.
#
#   impl/c/ci/smoke.sh <modbus|opcua|canbus|canopen|profibus|snmp>
#
# Expects (CI provides all of these):
#   * the protocol simulator running (docker compose -f
#     connectors/<proto>/docker-compose.yaml up -d simulator, plus vcan0 for
#     the SocketCAN protocols);
#   * an MQTT broker on 127.0.0.1:1883;
#   * impl/c/build/tedge-dot built;
#   * mosquitto_sub + jq on PATH.
#
# Asserts that a known seeded point arrives with quality "good" and the
# expected value, and (where the demo config has a writable point) that an
# MQTT write command round-trips with status "successful".
set -euo pipefail

proto=${1:?usage: smoke.sh <protocol>}
repo=$(cd "$(dirname "$0")/../../.." && pwd)
bin="$repo/impl/c/build/tedge-dot"
workdir=$(mktemp -d)
connector_pid=""
trap 'kill $connector_pid 2>/dev/null || true; rm -rf "$workdir"' EXIT

config="$workdir/$proto.toml"
cp "$repo/demo/config/$proto.toml" "$config"

# The demo configs get their points from the packaged point libraries (contract
# §3.4), which a checkout has not installed -- and the copy above moved the
# config away from them anyway, so a relative reference would not help either.
# Point the search path at the repo's copies.
export TEDGE_DOT_POINT_LIBRARY_PATH="$repo/demo/points.d"

# Per-protocol: expected point + value, optional write point/value, config fixups.
write_point="" write_value="" write_device="" trap_point="" trap_expect=""
case "$proto" in
modbus)
    point=temp_u16 expect=17001 device=plc1
    write_point=coil_rw write_value=true write_device=plc1
    ;;
opcua)
    point=temperature expect=21.5 device=opc1
    write_point=setpoint write_value=41 write_device=opc1
    ;;
canbus)
    point=rpm expect=2500 device=engine
    # demo config points at the installed demo path; use the repo copy
    sed -i.bak "s|/usr/share/tedge-dot/demo/can/test.dbc|$repo/connectors/canbus/sim/test.dbc|" "$config"
    ;;
canopen)
    point=analog_in expect=1234 device=plc1
    write_point=digital_out write_value=1 write_device=plc1
    ;;
profibus)
    point=ai0_raw expect=4660 device=remote_io
    write_point=do_byte0 write_value=5 write_device=remote_io
    ;;
snmp)
    # Polled from the pysnmp agent `just sim snmp` publishes on host port 1161 (INTEGER seed
    # 1234, connectors/snmp/agent/agent.py), with a SET round-trip on its writable INTEGER.
    point=pump_speed expect=1234 device=snmp-switch
    write_point=setpoint write_value=55 write_device=snmp-switch
    # And one notification: the traps come from this host's net-snmp (the simulator container
    # is idle unless told to send), to an unprivileged port on loopback, repeatedly until the
    # connector has had time to start.
    trap_point=pump_temperature trap_expect=85.3
    sed -i.bak 's|listen    = "0.0.0.0:162"|listen    = "127.0.0.1:1162"|' "$config"
    (for _ in $(seq 30); do
        snmptrap -m '' -v 2c -c public 127.0.0.1:1162 '' 1.3.6.1.4.1.99999.0.1 \
            1.3.6.1.4.1.99999.2.1 s "smoke" 1.3.6.1.4.1.99999.2.2 i 853 >/dev/null 2>&1 || true
        sleep 1
    done) &
    ;;
*)
    echo "unknown protocol: $proto" >&2
    exit 2
    ;;
esac

echo "== $proto: starting connector (30s window)"
"$bin" run "$config" --duration 30s 2>"$workdir/connector.log" &
connector_pid=$!

echo "== waiting for a good sample on te/device/$device/ot/$proto/sample/$point"
sample=$(mosquitto_sub -h 127.0.0.1 -W 25 -C 1 \
    -t "te/device/$device/ot/$proto/sample/$point" || true)
if [ -z "$sample" ]; then
    echo "FAIL: no sample received; connector log:" >&2
    cat "$workdir/connector.log" >&2
    exit 1
fi
echo "sample: $sample"
quality=$(jq -r .quality <<<"$sample")
value=$(jq -r .value <<<"$sample")
if [ "$quality" != "good" ] || [ "$value" != "$expect" ]; then
    echo "FAIL: expected quality=good value=$expect, got quality=$quality value=$value" >&2
    exit 1
fi
echo "OK: $point = $value (good)"
access=$(jq -r .access <<<"$sample")
if [ "$access" != "read" ] && [ "$access" != "read_write" ] && [ "$access" != "write" ]; then
    echo "FAIL: sample carries no valid access field (got '$access')" >&2
    exit 1
fi
echo "OK: sample echoes access=$access"

# A pushed point on the same device (SNMP: a notification next to the polled objects).
if [ -n "$trap_point" ]; then
    echo "== waiting for a good sample on te/device/$device/ot/$proto/sample/$trap_point"
    sample=$(mosquitto_sub -h 127.0.0.1 -W 15 -C 1 \
        -t "te/device/$device/ot/$proto/sample/$trap_point" || true)
    echo "sample: $sample"
    if [ "$(jq -r .quality <<<"$sample" 2>/dev/null)" != "good" ] || \
       [ "$(jq -r .value <<<"$sample" 2>/dev/null)" != "$trap_expect" ]; then
        echo "FAIL: expected $trap_point quality=good value=$trap_expect; connector log:" >&2
        cat "$workdir/connector.log" >&2
        exit 1
    fi
    echo "OK: $trap_point = $trap_expect (good)"
fi

if [ -n "$write_point" ]; then
    cmd_topic="te/device/$write_device/ot/$proto/cmd/write/smoke-1"
    echo "== write round-trip on $cmd_topic"
    mosquitto_pub -h 127.0.0.1 -t "$cmd_topic" -r \
        -m "{\"status\":\"init\",\"point\":\"$write_point\",\"value\":$write_value}"
    result=$(mosquitto_sub -h 127.0.0.1 -W 15 -t "$cmd_topic" | \
        jq -c --unbuffered 'select(.status == "successful" or .status == "failed")' | head -1 || true)
    echo "result: $result"
    if [ "$(jq -r .status <<<"$result")" != "successful" ]; then
        echo "FAIL: write command did not succeed; connector log:" >&2
        cat "$workdir/connector.log" >&2
        exit 1
    fi
    echo "OK: write $write_point = $write_value successful"
    # clear the retained command so reruns start clean
    mosquitto_pub -h 127.0.0.1 -t "$cmd_topic" -r -n

    # write-batch (contract §6.4): the same write plus an unknown point -> the batch
    # fails at the unknown point but reports the write that was applied before it.
    batch_topic="te/device/$write_device/ot/$proto/cmd/write-batch/smoke-2"
    echo "== write-batch round-trip on $batch_topic"
    mosquitto_pub -h 127.0.0.1 -t "$batch_topic" -r \
        -m "{\"status\":\"init\",\"writes\":[{\"point\":\"$write_point\",\"value\":$write_value},{\"point\":\"no_such_point\",\"value\":1}]}"
    result=$(mosquitto_sub -h 127.0.0.1 -W 15 -t "$batch_topic" | \
        jq -c --unbuffered 'select(.status == "successful" or .status == "failed")' | head -1 || true)
    echo "result: $result"
    if [ "$(jq -r .status <<<"$result")" != "failed" ] || \
       [ "$(jq -r '.results[0].status' <<<"$result")" != "successful" ] || \
       [ "$(jq -r '.results[1].status' <<<"$result")" != "failed" ] || \
       [ "$(jq -r '.results | length' <<<"$result")" != "2" ]; then
        echo "FAIL: write-batch did not stop at the unknown point with per-point results; connector log:" >&2
        cat "$workdir/connector.log" >&2
        exit 1
    fi
    echo "OK: write-batch applied $write_point then failed on no_such_point"
    mosquitto_pub -h 127.0.0.1 -t "$batch_topic" -r -n
fi

echo "== $proto smoke passed"
