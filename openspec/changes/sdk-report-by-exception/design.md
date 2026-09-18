## Context

Samples are published from a small, fixed set of functions in each runtime:

- **Rust**: `publish_sample` in `impl/rust/crates/sdk/src/runtime.rs`. Both the poll loop and the
  subscription channel call it. The main loop also wakes on a 200 ms `tick`. The stdout mode
  has its own loop, `run_stdout_until`, which publishes through `print_sample`.
- **C**: `emit_sample` in `impl/c/sdk/src/runtime.c`. Both the poll loop and `push_sink`
  (called through `drain_subscriptions` on the main loop thread) call it, and so does stdout
  mode. The loop computes `now = tdot_mono()` on every pass.

So the policy can be applied at these few call sites, with no change to any connector module.
Both runtimes already own `seq`, the device identity, and `meta` echoing.

`ot-measurement` implements `on_change`, `deadband`, `min_interval` and `debounce`
(`flows/ot-measurement/main.js:156-196`). Because it is a flow, it only reacts to incoming
messages and keeps its state per flow instance. That causes two things:

- it cannot run a heartbeat or send a held-back sample on a timer;
- the filtering applies only to measurements, so every other consumer still gets the full
  stream.

There is precedent for this move. `transform` is also a declared point property whose math the
SDK owns (contract §4.2), so the SDK owning the reporting policy fits the existing split.

## Goals / Non-Goals

**Goals:**
- One reporting policy for every connector (Modbus, OPC UA, SNMP, CAN, CANopen and PROFIBUS
  today, and future ones such as J1939 without extra work), with identical behaviour in Rust
  and C.
- Two new behaviours: a heartbeat (`max_interval`), and `min_interval` that sends the held-back
  change when the window ends instead of dropping it.
- A deadband that can be absolute or a percentage.
- A config field that is easy to find and documented, validated by the schema, and inherited
  from the connector, the device or a point library.

**Non-Goals:**
- Deadband filters on the server or protocol side, such as the OPC UA `DataChangeFilter`. They
  could be added later and derived from the same `report` table.
- Keeping the policy state across restarts. The first reading after a start is always
  published.
- Removing the flow-level settings in this change. They are only deprecated.
- Aggregation (min/max/avg over a window) and combining series. These stay in flows.

## Decisions

### D1: A first-class `report` table, not `meta`
Config shape:

```toml
[connector]
report = { max_interval = "15m" }                 # default for every point

[[device]]
report = { on_change = true }                     # device default

  [[device.point]]
  report = { deadband = 0.5, min_interval = "10s" }  # point override
```

Inheritance merges key by key. From least to most specific:

1. `[connector]`
2. `[[device]]`
3. the point as declared in a point library
4. the point as declared (or overridden) in the site's config

A more specific level wins for the keys it sets. A library point is point-level, so its
`deadband` wins over a site's device-level `deadband`. A site overrides it by setting `report`
on that point. In the Rust loader `report` joins `DEEP_MERGED_KEYS`
(`impl/rust/crates/sdk/src/library.rs:57`), and it joins the matching key list in the C loader
(`config.c`). Without that, a site's `report` would replace the library's table wholesale
instead of merging with it.

An inherited key can be switched off at a more specific level:

- `max_interval = "0"`, `min_interval = "0"` or `debounce = "0"` disables that setting;
- `on_change = false` together with `deadband = 0` disables change detection.

- Why not `meta`: the contract says the connector *never* interprets `meta`. Having the runtime
  read `meta.on_change` would break that rule, and the settings would stay hidden among
  free-form keys.
- Alternative considered: flat fields on the point (`on_change = true`). This was rejected
  because it clutters the point namespace and makes a device-level default awkward.

**Validation:**

- A single `report` table that sets both `min_interval` and `max_interval`, with
  `max_interval <= min_interval`, is rejected. So is a negative deadband, a malformed percent,
  or an invalid duration.
- The `max_interval <= min_interval` case can also arise only through inheritance, for example
  a point with `min_interval = "1h"` under the shipped `max_interval = "30m"`. That config is
  **not** rejected. The effective heartbeat is raised to twice `min_interval`, keeping it
  strictly greater as validation requires, and the loader logs a warning that names the
  point. The shipped default therefore never invalidates an existing
  config.

### D2: Semantics
Each point is evaluated independently, keyed by (device, point id). The runtime compares the
value **after** `transform`, which is what the sample's `value` carries. The policy consists of
the steps below, in order.

1. **Always publish**, bypassing every other rule, including `min_interval` and `debounce`,
   when any of these hold:
   - it is the first sample since the runtime started;
   - it is the first sample since the policy state was reset (see "Resets" below);
   - its `quality` differs from the last published sample's quality.

   Publishing a quality change also discards any pending (rate-limited) sample and any
   debounce candidate. An older good value can therefore never follow a newer bad one.
2. **Debounce** (`debounce > 0`, implies change detection):
   - A reading that differs from the last published value becomes the *candidate*.
   - Later readings that are "the same" as the candidate keep it alive. "The same" means
     within the deadband when one is set, and equal (epsilon 1e-9) otherwise, so noisy analog
     values can settle.
   - A reading that is not the same replaces the candidate and restarts the period. A reading
     that is not a change from the last published value clears the candidate.
   - The candidate is accepted once it has been the candidate for `debounce`, measured on the
     monotonic clock.
   - The published sample is the **most recent** reading of the stable run, with its own `ts`.
   - A pushed point that receives no further push is accepted by the loop once the period ends.
3. **Change test** (`on_change` or `deadband`):
   - Numbers (`value_repr = "number"`): publish if `|v - last| >= threshold`.
     - An absolute deadband's threshold is the configured number.
     - A `"<p>%"` deadband's threshold is `p/100 × |last|`. With `last == 0`, any non-zero
       change passes.
     - With `on_change` and no deadband, any difference greater than 1e-9 passes.
   - NaN counts as a value equal only to NaN. NaN → NaN is not a change. A number → NaN, or
     NaN → a number, is a change. This keeps a published NaN from blocking the point forever.
   - Anything else is compared by exact equality of `(value_repr, value)`, or of `raw` when
     there is no decoded value, and `deadband` is ignored. This covers bool, string, bytes,
     raw mode, and 64-bit integers carried as strings (contract §4.1). If a 64-bit integer
     switches between number and string representation, that counts as a change.
4. **Rate limit** (`min_interval`): if the last publish was less than `min_interval` ago, the
   sample is stored as *pending*, replacing any older pending sample, and is not published.
   Once the window has elapsed, the next pass of the main loop re-runs the change test on the
   pending sample and publishes it if it still passes. The sample keeps its original `ts`.
5. **Heartbeat** (`max_interval`): a heartbeat **never fabricates a sample**. When a point has
   published nothing for `max_interval`, its next *fresh reading* is published even if it has
   not changed:
   - **Polled point**: this is simply the next scheduled read.
   - **Pushed point**: the main loop requests an on-demand read of it through the ordinary
     `read_points` / `read_point` path. That read also proves the source is alive.
   - A heartbeat read has three possible outcomes:
     - **Sample**: the reading is published, even when unchanged. A bad sample is published as
       a quality change.
     - **Failure**: in Rust, `read_points` returns an `Err` other than `Unsupported`, or the
       operation timeout expires. In C, `read_point` returns `-1`. The runtime synthesises a
       bad-quality sample for the point, with the error as its reason, and publishes it. A
       hung or unreachable source is therefore reported, not silently skipped.
     - **No data**: Rust returns `Unsupported` or an `Ok` without a sample for the point. C
       returns the new `TDOT_READ_NO_DATA` (see D3). The point gets no heartbeat, and it is
       marked *unreadable* until its next reset, so it is not retried on every pass. Examples
       are SNMP trap and varbind points and CAN frames.
   - Side effects: heartbeat reads feed the device's link status exactly like polls, through
     `note_poll` in Rust and the rc handling in C. A transport failure therefore schedules the
     same reconnect, and it also resets the push subscription, as a failed poll does today.
     This is intended: a failed heartbeat read *is* evidence the device is unreachable.
   - Each point gets at most one heartbeat read per `max_interval`. The reads are grouped per
     device and bounded by the operation timeout, as polls are.
6. `last` is updated only when a sample is published. The deadband is measured from the last
   published value, so slow drift is still reported once it adds up to the deadband. This
   matches the flow.
7. `seq` is stamped only when the policy decides to publish. A sample that is then lost
   because the broker is unreachable (Rust's `!is_online()` drop) still consumes its `seq`, as
   today. A gap in `seq` therefore still means "published but lost", and never "filtered".

**Resets**: the whole runtime's policy state (for every point) is cleared in each of these
cases, so the next sample of every point is published:

- an applied reload or configuration management command, which today already clears the
  `seq` counters and reconnects the devices;
- a device reconnect (for that device's points);
- an **MQTT session restore after a broker reconnect**. Without this, `last` could hold a value
  that never reached the broker: Rust drops samples while offline, and C's `mosquitto_publish`
  fails silently. A filtered point would then stay quiet after the broker came back.

- Why a percentage of the last published value, and not of a declared engineering range:
   - It needs no extra config.
   - It matches the "relative deviation" setting that historians use.
   - Its weakness near zero is covered by rule 3: with `last == 0`, any change passes.
- Alternative considered: OPC UA's percent-of-EURange. That needs `range = [lo, hi]` on every
  point. It is deferred (see D6).
- Alternative considered for the heartbeat: replaying the last value with a fresh `ts`. This
  was rejected in review for two reasons:
  - The runtime cannot tell whether a pushed source is alive. canbus has no
    `check_subscription`, and for devices that are both polled and pushed only the reads judge
    the link.
  - Replaying an *occurrence* point (an SNMP trap with `meta.event.every`) would raise
    invented events every `max_interval`.

### D3: Where it lives in each runtime
Each runtime has a pure policy module with the same interface, and each place that currently
publishes a sample goes through it.

- **Rust**: a new `report` module in the SDK crate provides a pure `ReportState`, a state
  machine per point, with:
  - `offer(sample, now) -> Decision`;
  - `due(now) -> Due`, which returns the pending or debounced samples that are ready to
    publish, and the points whose heartbeat read is due.

  The pure design allows proptest without MQTT or a real clock. The hooks:
  - `offer` is called by `publish_sample` (both the poll path and the push path) **and by
    `print_sample`** in the separate stdout loop `run_stdout_until`.
  - `due` is called on every loop tick. Heartbeat reads go through `bounded(read_points)`.
  - A reset is triggered on reload, on device reconnect, and in `restore_mqtt_session`.
- **C API change**: the `read_point` contract (`include/tedge_dot/connector.h`) gains a third
  return value, `TDOT_READ_NO_DATA`. It means "this point has nothing to read on demand", and
  the runtime ignores `*out`. Today a C module can only return a bad sample, which would
  publish bad quality on every heartbeat.
  - `connector_snmp.c` returns it for trap and varbind points, instead of the current bad
    sample with rc 0.
  - `connector_canbus.c` (which polls a frame cache) returns it when no new frame for the CAN
    id has arrived since the point's previous read. A cached frame is then never published
    twice as if it were a fresh reading. This matches Rust canbus, which publishes once per
    received frame.
  - The runtime treats `TDOT_READ_NO_DATA` on a normal poll the same way: nothing is
    published, and the link is unaffected.
- **C**: `sdk/src/report.c` provides `tdot_report_offer` and `tdot_report_due`, with the state
  stored on `tdot_point_t`: last value and quality, time of the last publish, the pending
  sample, and the debounce candidate. The hooks:
  - `emit_sample` calls `tdot_report_offer`. It serves the poll path, the push path via
    `push_sink`, and stdout mode.
  - The main loop calls `tdot_report_due` on each pass.
  - A reset is triggered in `commit_config`, on device reconnect, and in the MQTT connect
    callback when a session is resumed.
- **Timing**: a trailing or debounced sample is published on the first loop pass after its
  window ends. A loop pass waits for the protocol reads due on that pass, which are bounded by
  `operation_timeout`. So the delay is "one loop iteration", not a fixed 200 ms.
- **Not affected**: the CLI `read` and `write` commands call the module directly (Rust
  `main.rs`, C `main.c`) and are exempt. They always print what they read.
- **Shared test vectors**: both implementations run one set of table-driven vectors in
  `doc/contract/test-vectors/report/`. Each vector gives a policy, a sequence of timed inputs,
  and the expected publishes. Input kinds: sample, tick, link reset, mqtt reset, and read
  result or unsupported. The Rust and C unit tests both run them.

### D4: Descriptor
The capability descriptor (§7) keeps its existing array style. It gains a `reports` object:

```json
"reports": {
  "default": { "max_interval": "30m" },
  "devices": [ { "device": "plc-1", "report": { "on_change": true } } ],
  "points":  [ { "device": "plc-1", "point": "temp", "report": { "on_change": true, "deadband": 0.5, "max_interval": "30m" } } ]
}
```

- `default` and `devices` hold what is declared at those levels.
- `points` lists the **effective** policy (after merging) only for points whose policy differs
  from their device's effective policy. The message stays small when only defaults are used.
  Contract §7 already warns about the descriptor's size.
- A consumer finds a point's effective policy from its `points` entry if it has one.
  Otherwise it merges its device's `devices` entry over `default`, key by key.
- The schema lives in `status.schema.json`.

### D5: `ot-measurement` compatibility
The flow keeps its filtering code, but the defaults stay off, so nothing is filtered twice
unless a user configures both. The docs mark the flow's settings as deprecated and point to
`report`.

- Alternative considered: have the runtime also read `meta.on_change` and the other `meta`
  keys. This was rejected (see D1), and the runtime and the flow would then both filter the
  same samples.

### D6: Default heartbeat in the shipped configs; percent-of-range deferred
- These configs set a connector-wide heartbeat that is active, not commented out:
  - the configs installed by the packages: `packaging/config/{modbus,opcua,canopen}.toml` and
    `impl/c/packaging/config/profibus.toml`;
  - the demo configs for the same protocols: `demo/config/*.toml`.

  ```toml
  [connector]
  # Publish every readable point at least this often, even when its value has not changed, so
  # the cloud keeps seeing the device (Cumulocity marks it unavailable after its required
  # interval). Points that cannot be read on demand (traps, CAN frames) are not affected.
  report = { max_interval = "30m" }
  ```

  - 30 minutes stays inside the 60-minute required interval used by the demos and by typical
    Cumulocity setups.
  - What it changes: a polled point without a filter already publishes every reading, so it
    is unaffected. The default matters for pushed points with static values (the OPC UA
    "offline" case, handled by the on-demand read) and for points a user gives a change
    filter.
  - **canbus and snmp** get the setting only as a commented-out example, because a heartbeat
    cannot apply to their push-only or trap points. Writing it there would suggest it does
    something.
  - Only fresh installs get the default. The packaged file is a conffile, so an edited
    `/etc` config keeps its content on upgrade. The release notes tell existing users how to
    add the setting.
  - The hard-coded default in the SDK stays "no heartbeat", so a config without `report`
    behaves exactly as before.
- Percent of a declared engineering range (`deadband_range = [lo, hi]`, as in OPC UA's
  PercentDeadband) is **deferred**. It will be added later only if users ask for it. The
  `deadband` parsing is written so that a sibling key can be added without breaking anything.

## Risks / Trade-offs

- [Every consumer sees the filtered stream. Alarm and event flows can react later, or not at
  all, to a change smaller than the deadband.] → Document that the deadband should be smaller
  than an alarm's hysteresis. The config loader warns when a point has both
  `meta.event.every` and a change filter or debounce in its effective `report`. This is a
  lint on free-form metadata only, and it never changes behaviour.
- [A heartbeat could hide a stale source.] → It cannot, because it is always a fresh read
  (D2.5). A failed read publishes bad quality.
- [Heartbeat reads add protocol traffic.] → At most one read per point per `max_interval`, and
  only for pushed points. This is negligible at 30 minutes.
- [State lost on restart, reload or broker reconnect means one extra publish per point.] →
  This is accepted and documented.
- [The C and Rust implementations could drift.] → Shared test vectors (D3), plus conformance
  cases run through the parity harness.
- [Trailing and debounced samples are sent late, with their original `ts`.] → Consumers
  already order by `ts`. The delay is at most the window plus one loop iteration.

## Migration Plan

- Opt-in, with no change for existing configs. Fresh installs get the 30-minute default
  heartbeat (D6).
- Users of `meta.on_change` and the other `meta` keys move to `report` by renaming the table.
  The migration guide and the RFC 0002 Cloud Fieldbus mapping (`noUpdateIfEqual` →
  `report.on_change`) are updated.
- Rollback: remove `report` from the config. The flow's settings are still available.

## Open Questions

- None. The heartbeat default was decided in D6, and percent of range was deferred.
