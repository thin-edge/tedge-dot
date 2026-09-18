# Migration guide: from the Python plugin to OT connectors + flows

This proposal is a **greenfield** design — it does not preserve the Python plugin's internals
or wire format. This guide shows existing users how their current setup maps onto the new
world, and sketches tooling to automate the move. The goal is that no information is lost: every
field in today's `devices.toml`/`modbus.toml` and every Cumulocity operation has a clear new
home.

## 1. The big picture

```text
            OLD (one Python plugin)                         NEW (driver + flows)
 ┌──────────────────────────────────────┐      ┌────────────────────────────────────────────┐
 │ tedge_modbus.reader                  │      │ tedge-dot (Rust, modbus module)            │
 │   transport + decode + scale + alarm │ ───▶ │   transport + primitive decode only        │
 │   + thin-edge JSON + registration    │      ├────────────────────────────────────────────┤
 │ tedge_modbus.operations (c8y_*)      │      │ flows: scaling, alarm, registration, …     │
 └──────────────────────────────────────┘      │ command verb `write` + thin-edge operations│
                                               └────────────────────────────────────────────┘
```

One Python responsibility becomes either a **connector config field** (transport/decode) or a
**flow** (everything with business meaning).

## 2. Config mapping

### 2.1 `modbus.toml` → connector config

| Legacy `modbus.toml` | New location |
| --- | --- |
| `[modbus].pollinterval` | `connector.poll_interval` (duration string, e.g. `"2s"`) |
| `[modbus].loglevel` | `connector.log_level` |
| `[modbus].combinemeasurements` | **flow** (a grouping/aggregation flow) |
| `[serial].*` (port, baudrate, …) | `connection.serial.*` (defaults) and/or per-device `protocol_address` |
| `[thinedge].mqtthost/mqttport` | `mqtt.host` / `mqtt.port` |
| `[thinedge].subscribe_topics` | implicit — the SDK derives command topics from the contract |

### 2.2 `devices.toml` → connector config + flows

A legacy device:

```toml
[[device]]
name = "TestCase1"
address = 1
ip = "simulator"
port = 502
protocol = "TCP"
littlewordendian = false
```

becomes:

```toml
[[device]]
name = "TestCase1"
protocol_address = { transport = "tcp", host = "simulator", port = 502, unit_id = 1 }
default_mode = "typed"
```

(`littlewordendian = false` → per-point `word_order = "big"`.)

A legacy register:

```toml
[[device.registers]]
number = 3
startbit = 0
nobits = 16
signed = true
multiplier = 1
divisor = 1
decimalshiftright = 0
offset = -20
input = false
datatype = "float"            # or absent for integer
name = "Test_Int16"
measurementmapping.templatestring = "{\"Test\":{\"Int16\":%%}}"
```

splits into **driver** fields and a **flow**:

Connector point (driver — *decode only*):

```toml
[[device.point]]
id       = "Test_Int16"
mode     = "typed"
datatype = "float32"           # legacy "float" with 16-bit float ≈ float32; integers → int16/uint16
endianness = "big"
word_order = "big"
address  = { table = "holding", address = 3, count = 2 }   # input=false → holding; count per datatype
# startbit/nobits → address.start_bit / bit_count when a bit-field is needed
```

Scaling + naming flow (`modbus-scaling` params for this point):

```toml
point        = "Test_Int16"
multiplier   = 1
divisor      = 1
offset       = -20             # legacy offset
# decimalshiftright = d  →  multiplier_effective = multiplier * 10^d  (fold into multiplier)
target_topic = "te/device/TestCase1///m/Test"
group        = "Test"
series       = "Int16"         # from the templatestring shape {"Test":{"Int16":%%}}
```

> The legacy scaling formula `(raw * multiplier * 10^decimalshiftright / divisor) + offset`
> maps exactly onto the [`modbus-scaling`](../flows/modbus-scaling/) flow by folding
> `10^decimalshiftright` into `multiplier`. The `templatestring` `{"Test":{"Int16":%%}}` maps to
> `group = "Test"`, `series = "Int16"`.

A legacy coil with an alarm/event:

```toml
[[device.coils]]
number = 2
input = false
alarmmapping.severity = "MAJOR"
alarmmapping.text = "Alarm triggered"
alarmmapping.type = "TestAlarm"
eventmapping.type = "TestEvent"
eventmapping.text = "Event triggered"
```

becomes a connector point (read the coil) plus an [alarm flow](../flows/modbus-alarm/) (or a
small event flow) keyed on that point/series:

```toml
[[device.point]]
id      = "TestAlarm_coil"
mode    = "typed"
datatype = "bool"
address = { table = "coil", address = 2, count = 1 }
```

```toml
# modbus-alarm params (adapted to read the bool sample directly, or a passthrough measurement)
alarm_type = "TestAlarm"
severity   = "major"
text       = "Alarm triggered"
```

## 3. Operations mapping

Every legacy operation maps onto a **generic, protocol-neutral command verb** (contract §6) driven
over MQTT. A thin Cumulocity shim translates the cloud operation into an `ot_<verb>` thin-edge
command (see [`tedge-dot/operations/`](../../../tedge-dot/operations/)), the
command bridge flows forward it to the connector, and the result is mirrored back so the cloud
operation completes.

| Legacy operation (Python) | Generic command (verb) | Implemented by |
| --- | --- | --- |
| `c8y_SetRegister` | `ot_write` (`write`) | protocol module — typed/raw point write |
| `c8y_SetCoil` | `ot_write_coil` (`write-coil`) | protocol module — coil write (alias for `write`; separate command type required by thin-edge.io) |
| `c8y_ModbusConfiguration` (poll rate) | `ot_set_config` (`set-config`) | SDK runtime — patches `connector.poll_interval` |
| `c8y_SerialConfiguration` | `ot_set_config` (`set-config`) | SDK runtime — patches `connection.serial.*` |
| `c8y_ModbusDevice` (+ `c8y_Coils`/`c8y_Registers`) | `ot_define_device` (`define-device`) | SDK runtime — adds device + points; child registration via the [registration flow](../flows/) |

The **command contract** (contract §6) replaces the bespoke `set_register.py`/`set_coil.py` logic
and the config-mutating operation handlers: `write` is implemented by the protocol module, while the
`set-config`/`define-device`/`remove-device` management verbs are implemented once in the SDK
runtime (it owns the connector configuration document — patch, validate, persist, live-reload). The
Cumulocity-specific operation templates become thin shims that translate a C8y operation into an
`ot_<verb>` command, plus flows for the data side.

## 4. Topic changes (what moves on the wire)

| Concern | Legacy topic | New topic |
| --- | --- | --- |
| Measurement | `te/device/<d>///m/<TYPE>` (driver) | same — but produced by a **flow** from `…/ot/modbus/sample/<point>` |
| Alarm | `te/device/<d>///a/<TYPE>` (driver) | same — produced by the **alarm flow** |
| Event | `te/device/<d>///e/<TYPE>` (driver) | same — produced by an **event flow** |
| Raw read | (none) | `te/device/<d>/ot/modbus/sample/<point>` (new) |
| Write command | `te/device/+///cmd/modbus_SetRegister/+` | `te/device/<d>/ot/modbus/cmd/write/<id>` |
| Registration | `te/device/<d>//` (driver) | same — produced by the **registration flow** |

The cloud-facing data model (`m`/`e`/`a`) is unchanged, so dashboards and cloud mappers keep
working; only the *producer* of those messages moves from the driver to flows.

## 5. Migration tooling concept

A one-shot `tedge-ot-migrate` helper (or a `just migrate` target) automates most of §2–§3:

```sh
tedge-ot-migrate \
  --modbus-toml /etc/tedge/plugins/modbus/modbus.toml \
  --devices-toml /etc/tedge/plugins/modbus/devices.toml \
  --out-dir ./migrated
```

It emits:

```text
migrated/
├── ot/modbus.toml                 # connector config (devices + points, decode-only)
├── flows/
│   ├── <device>-<point>-scaling/  # one scaling flow per measurement-mapped register
│   │   ├── flow.toml
│   │   ├── main.js                # symlink/copy of modbus-scaling
│   │   └── params.toml            # filled from templatestring + scaling fields
│   └── <device>-<point>-alarm/    # one alarm flow per alarm-mapped coil/register
└── MIGRATION_NOTES.md             # anything that needs human review (exotic datatypes, etc.)
```

Translation rules the tool applies:

- `input=false`→`holding`, `input=true`→`input` (registers); coils similarly.
- `datatype="float"`+width → `float32`/`float64`; absent → `int16`/`uint16` per `signed`,
  widening to `int32`/`uint32` when `nobits > 16`.
- `littlewordendian`→`word_order`; per-register `littleendian`→`endianness`.
- `startbit`/`nobits` (partial register) → `address.start_bit`/`bit_count` (requires the
  connector's `bitfield` feature).
- Fold `decimalshiftright` into the flow `multiplier`.
- `measurementmapping.templatestring` → flow `group`/`series`/`target_topic` by parsing the
  JSON template (`%%` is the value slot).
- `alarmmapping`/`eventmapping` → alarm/event flow params.
- `combinemeasurements` → emit a grouping flow note (manual review).

Anything the tool can't translate confidently (unusual templatestrings, nested combine logic)
is written to `MIGRATION_NOTES.md` for human review rather than guessed.

## 6. Recommended cut-over

1. Run `tedge-ot-migrate` to produce config + flows.
2. Validate offline: `tedge flows test` each generated flow against representative samples;
   run the connector against the [simulator](../../connectors/modbus/sim/) with the new config.
3. Deploy the connector and flows on a staging device; compare cloud data with the legacy
   plugin side-by-side.
4. Disable the Python plugin's service; enable `tedge-dot`.
5. Remove the Python package once satisfied.

Because the cloud-facing topics are unchanged, the cut-over is observable and reversible at the
service level.

## 7. Upgrading tedge-dot: OPC UA secured connections (BREAKING)

From this release both builds connect to secured OPC UA servers as a supported feature
([OPC UA connector spec](../connectors/opcua-connector-spec.md)). Configurations that use
`security_policy = "None"` (the default) are **not affected**: they behave as before, need no
certificate and create no PKI directory.

**What breaks.** The Rust build used to accept *every* server certificate. It no longer does:
a server certificate is trusted only when it is pinned (its exact copy is in
`trusted/certs/`) or when it was issued by a CA in `trusted/certs/` whose CRL is present. (A
secured policy did not actually work before this release — the Rust build never had an
application certificate, and the C build refused anything but `None` — so the practical
impact is on configurations written in anticipation.)

**First contact with a secured server:**

1. On its first start with a secured device, the connector generates its application
   certificate in `/var/lib/tedge-dot/opcua/pki/own/` (or uses `certificate`/`private_key`
   from `[connection]`).
2. The server's certificate is unknown, so the device stays `disconnected` with the link-status
   reason `certificate untrusted: … thumbprint <hex> …`, and the certificate is copied to
   `rejected/certs/`.
3. Check it and trust it — no restart needed:

   ```sh
   tedge-dot pki list rejected
   tedge-dot pki trust <first 8 digits of the thumbprint>
   ```

4. Most servers also have to trust the connector. Give the server administrator its
   certificate:

   ```sh
   tedge-dot pki export --pem --output tedge-dot.pem
   ```

   Until they do, the reason reads `application certificate rejected: the server rejected the
   connection …`. (Before this category existed it read `certificate untrusted: …`, which is the
   prefix an older gateway reports.)

**Trusting a site CA instead of each server.** Import the CA certificate and its CRL; every
server certificate it issues is then trusted:

```sh
tedge-dot pki trust site-root-ca.pem          # a CA certificate becomes a trust anchor
tedge-dot pki add-issuer site-issuing-ca.pem  # intermediates, if any
tedge-dot pki add-crl site-root-ca.crl        # one per CA: without it, certificates are refused
tedge-dot pki add-crl site-issuing-ca.crl
```

A CA **without a CRL** makes every certificate it issued fail with `certificate revoked: …
revocation unknown`. If the CA has never revoked anything, publish (or ask for) an empty CRL.
`tedge-dot pki list` shows `NO CRL` for such a CA.

**Other changes to check:**

- A password on a channel without message security is refused, even when the server offers
  token encryption (no server certificate is authenticated on such a channel), as is one sent
  on a signed-only channel whose token policy is `None` (`plaintext password refused:`). Use
  `sign_and_encrypt`, or set `allow_plaintext_password = true` only if the server offers
  nothing better.
- `Basic128Rsa15` and `Basic256` need `allow_deprecated_security = true`.
- `security_mode = "none"` with a secured policy (and a secured mode with `None`) is now a
  configuration error instead of being silently ignored. An unknown policy or mode is an
  error too.
- Keep passwords out of the configuration with `password_file`.
- As a temporary escape hatch, `trust_any_server_certificate = true` restores the old
  behaviour for a device or the whole connection (messages are still signed/encrypted; a
  warning is logged on every connect). Do not leave it on.
