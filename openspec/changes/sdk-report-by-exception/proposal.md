## Why

Users want to send a signal only when it changes, or when it changes by more than a threshold
(a deadband). They also want a signal that is flat to still send a periodic sign of life.
Today this exists only inside the `ot-measurement` flow, and it is configured through undocumented
keys in the free-form `meta` table (`meta.on_change`, `meta.deadband`, `meta.min_interval`,
`meta.debounce`). Those keys are listed only as examples in the contract and in the flow's
`params.toml.template`. Users and AI assistants did not find the feature at all.

Doing the filtering in one flow also has these problems:

- Every other consumer of the samples (alarms, events, the parameter twin, third-party tools)
  still gets every reading.
- Nothing is saved on the device's MQTT broker.
- The flow reacts only to incoming messages, so it cannot:
  - send a heartbeat for a value that stays the same, or
  - send the last change it held back once a rate-limit window ends.

These two gaps were found in the flow's behaviour:

1. With `on_change` or `deadband` set, a flat signal is never sent again. In Cumulocity the
   device then goes unavailable once its required interval expires.
2. `min_interval` discards a reading that arrives too early instead of holding it. For push
   sources (OPC UA subscriptions, traps) the final change in a burst can therefore be lost
   until the value changes again.

The deadband also accepts only an absolute value. There is no relative (percent) form.

## What Changes

- Add a new first-class point field, **`report`**. It is owned by the SDK runtime in both the
  Rust and C implementations and applied to every connector at the few call sites where samples
  are published. A device and the `[connector]` section can set a default for it, the same way
  they do for `poll_interval`. Point libraries can declare it too. The merge goes key by key,
  and a more specific level can switch an inherited setting off with `"0"`.
  - `on_change` (bool): publish only when the value differs from the last published one.
  - `deadband` (number, or string `"<n>%"`): the minimum absolute change, or the minimum
    change relative to the last published value, needed to publish. Setting it implies
    `on_change`.
  - `min_interval` (duration): publish at most once per window per point. A change held back
    during the window **is published when the window ends** (trailing edge). It is not
    dropped.
  - `max_interval` (duration, **new**): a heartbeat. If a point has not been published for
    this long, its next *fresh reading* is published even though it has not changed. For a
    polled point this is the next poll. For a pushed point the runtime reads it on demand,
    which also proves the source is alive. The runtime never replays an old value. Points that
    cannot be read on demand, such as SNMP traps and CAN frames, get no heartbeat.
  - `debounce` (duration): a changed value is published only after it has stayed stable this
    long.
  - A change in `quality` (including recovering from an error back to a good reading) and the
    first reading after a start, a reload, a device reconnect or a broker reconnect are
    **always** published.
  - `seq` counts only published samples, so a gap in `seq` still means a sample was lost.
- Amend the contract: §3.1 gets the new point field, and a new section describes the reporting
  policy. The line saying the driver MUST NOT apply thresholding is clarified: the **SDK
  runtime** applies the declared reporting policy, just as it applies `transform`. The
  connector module still MUST NOT filter on its own.
- Update the JSON schemas and the capability descriptor. The descriptor publishes a `reports`
  object: the connector default, the device policies, and each point that differs from its
  device.
- `ot-measurement`: its `on_change`, `deadband`, `min_interval` and `debounce` settings (flow
  parameters and `meta.*`) become **deprecated**. They keep working, and the docs point to
  `report` instead.
- Add conformance tests (layer 3) that run against both implementations through the parity
  harness, including push-based and heartbeat scenarios.
- The packaged and demo configs for modbus, opcua, canopen and profibus set a connector-wide
  heartbeat, `[connector] report = { max_interval = "30m" }`. Devices whose values never change,
  such as OPC UA subscriptions to static nodes, then stay available in Cumulocity. The canbus
  and snmp configs show the setting only as a commented-out example, because their push-only
  points cannot be read on demand. The SDK itself still defaults to no heartbeat.
- Deferred: a percent-of-range deadband (`deadband_range`, like OPC UA's PercentDeadband). It
  will be added later only if needed.
- Documentation, so users can find the feature:
  - a "Reducing data volume" section in the user-facing docs;
  - references from each connector spec, the flows README and the migration guide;
  - updated demo configs that use `report`.

## Capabilities

### New Capabilities
- `point-reporting-policy`: SDK-runtime report-by-exception for connector samples. This covers
  on-change, absolute and percent deadband, rate limiting with trailing emit, heartbeat, and
  debounce, along with their inheritance, how they interact with quality, `seq` and reconnects,
  and how they are configured and documented.

### Modified Capabilities
<!-- None: no existing openspec spec covers sample publishing or ot-measurement. -->

## Impact

- **Contract and schemas**: `doc/contract/ot-connector-contract.md` (§3.1, §4, new reporting
  section, §7 descriptor), `doc/contract/schemas/config.schema.json`, and
  `doc/contract/schemas/{sample,point-library,status}.schema.json` (the `meta` description, the
  library `report`, and the descriptor's `reports`).
- **Rust SDK**: `impl/rust/crates/sdk/src/{config.rs,runtime.rs,descriptor.rs,library.rs}` and a
  new `report.rs`. The policy state machine sits in front of `publish_sample` and
  `print_sample`, and the existing tick sends trailing samples and runs heartbeat reads.
- **C SDK**: `impl/c/sdk/src/{config.c,runtime.c,descriptor.c}`, a new `report.c`, and `include/tedge_dot/config.h`.
  The state lives on each point (`tdot_point_t`), and `emit_sample` and the main loop apply it.
- **C module API**: `read_point` gains a `TDOT_READ_NO_DATA` return value. The C SNMP module
  returns it for trap and varbind points, and C canbus when no new frame has arrived, so a
  cached frame is no longer published again as a fresh reading. This brings C canbus in line
  with Rust, which publishes once per frame.
- **Conformance and e2e**: `impl/rust/crates/ot-conformance` (layer-3 cases), the parity harness,
  and the OPC UA and Modbus e2e suites and conformance configs that use `meta.on_change`.
- **Flows**: `flows/ot-measurement` (deprecation notes; the behaviour stays),
  `flows/test-flows.sh`.
- **Docs and demo**: the contract, `flows/README.md`, `doc/connectors/*-spec.md`,
  `doc/migration/*`, RFC 0002 (the send-on-change mapping), `demo/**`, `packaging/config/*`,
  `packaging/release-notes.md`.
- **Not affected**: the CLI `read` and `write` commands, which print or perform every read or
  write as before.
- **Behaviour**: no change for existing configs. `report` is opt-in, and when it is absent
  every reading is published as today. Every consumer of samples on a point that sets
  `report` sees the filtered stream. See the design for what this means for alarms.
