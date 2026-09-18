## Context

Samples are published from exactly one function in each runtime:

- **Rust**: `publish_sample` in `impl/rust/crates/sdk/src/runtime.rs`. Both the poll loop and the
  subscription channel call it. The main loop also wakes on a 200 ms `tick`.
- **C**: `emit_sample` in `impl/c/sdk/src/runtime.c`. Both the poll loop and `push_sink`
  (called through `drain_subscriptions` on the main loop thread) call it. The loop computes
  `now = tdot_mono()` on every pass.

So the policy can be applied in one place per runtime, with no change to any connector module.
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

Inheritance merges key by key: a point's value wins over the device's, and the device's wins
over the connector's. A point library can declare `report` on its points, and the site's
point-level value wins over the library's. This uses the same rules as the existing library
field merge (RFC 0004).

- Why not `meta`: the contract says the connector *never* interprets `meta`. Having the runtime
  read `meta.on_change` would break that rule, and the settings would stay hidden among
  free-form keys.
- Alternative considered: flat fields on the point (`on_change = true`). This was rejected
  because it clutters the point namespace and makes a device-level default awkward.

### D2: Semantics
Each point is evaluated independently, keyed by (device, point id). The runtime compares the
value **after** `transform`, which is what the sample's `value` carries.

1. **Always publish** when any of these hold. These cases bypass every filter except
   `min_interval`, and they bypass that too when quality changes:
   - it is the first sample since the runtime started, since a reload that changed this
     point's config, or since the device reconnected;
   - `quality` differs from the last published sample's quality.
2. **Debounce** (`debounce > 0`): a candidate value is accepted only once it has been seen
   unchanged for `debounce`. The age is measured on the monotonic clock. For a pushed point
   with no further push, the tick sends the value once the period has passed.
3. **Change test** (`on_change` or `deadband`):
   - Numbers: publish if `|v - last| >= deadband`. For an absolute deadband, `deadband` is the
     number. For `"<p>%"` it is `p/100 * |last|`. With `last == 0` any non-zero change passes.
     With `on_change` and no deadband, any difference passes, using an epsilon of 1e-9 as the
     flow does.
   - bool, string, bytes and raw mode: publish on any difference in `value`, or in `raw` when
     there is no decoded value. `deadband` does not apply to them.
4. **Rate limit** (`min_interval`): if the last publish was less than `min_interval` ago,
   store the sample as *pending* (replacing any older pending one) and do not publish. When
   the tick finds that the window has elapsed, it re-runs the change test on the pending
   sample and publishes it if it still passes. The sample keeps its original `ts`.
5. **Heartbeat** (`max_interval`): if a point has published nothing for `max_interval`:
   - Polled point: the next reading is published whether or not it changed. No timer is
     needed, because a reading arrives every `poll_interval`.
   - Pushed point: the tick re-publishes the last published sample with `ts = now`, but only
     while the device's link is `connected`. A heartbeat must never make a dead source look
     alive.
   - A sample sent as a heartbeat is not marked specially, because to a consumer it is simply a
     current reading.
   - The configuration is rejected if `max_interval <= min_interval`.
6. `last` is updated only when a sample is published. The deadband is measured from the last
   published value, so slow drift is still reported once it adds up to the deadband. This
   matches the flow.
7. `seq` is incremented only on publish (both runtimes move the increment after the policy
   decision). A gap in `seq` still means a published sample was lost.

- Why a percentage of the last published value, and not of a declared engineering range:
   - It needs no extra config.
   - It matches the "relative deviation" setting that historians use.
   - Its weakness near zero is covered by rule 3: with `last == 0`, any change passes.
- Alternative considered: OPC UA's percent-of-EURange. That needs `range = [lo, hi]` on every
  point. It is deferred (see D6).

### D3: Where it lives in each runtime
- **Rust**: a new `report` module in the SDK crate. It provides a pure `ReportState` (a state
  machine per point) with `offer(sample, now) -> Decision` and `due(now) -> Vec<Sample>` (the
  trailing, debounced and heartbeat samples). Both publish paths call `offer`, and the tick
  loop calls `due`. Keeping the state machine pure lets it be tested with proptest, without
  any MQTT or a clock.
- **C**: `sdk/src/report.c` with the same functions, operating on state stored in
  `tdot_point_t` (last value, last quality, last publish time, pending sample, candidate).
  `emit_sample` calls `tdot_report_offer`, and the main loop calls `tdot_report_due` on each
  pass.
- Both implementations share one set of table-driven test vectors: a JSON file of
  (config, sequence of timed samples, expected publishes) in
  `doc/contract/test-vectors/report/`. Rust unit tests and C unit tests both run it, which
  keeps the two implementations in step at the unit level before the conformance suite runs.

### D4: Descriptor and link status
The capability descriptor (§7) gains `point_reports`: a map from point id to its
**effective** policy, after inheritance. It includes only points that have a policy. This
lets tooling and flows see why a point is quiet, without reading the connector config.

### D5: `ot-measurement` compatibility
The flow keeps its filtering code, but the defaults stay off, so nothing is filtered twice
unless a user configures both. The docs mark the flow's settings as deprecated and point to
`report`.

- Alternative considered: have the runtime also read `meta.on_change` and the other `meta`
  keys. This was rejected (see D1), and the runtime and the flow would then both filter the
  same samples.

### D6: Default heartbeat in the shipped configs; percent-of-range deferred
- The configs installed by the packages (`packaging/config/*.toml` and
  `impl/c/packaging/config/profibus.toml`) and the demo configs (`demo/config/*.toml`) set a
  connector-wide heartbeat, active rather than commented out:

  ```toml
  [connector]
  # Re-publish every point at least this often, even when its value has not changed, so the
  # cloud keeps seeing the device (Cumulocity marks it unavailable after its required interval).
  report = { max_interval = "30m" }
  ```

  - 30 minutes stays inside the 60-minute required interval used by the demos and by typical
    Cumulocity setups.
  - On its own, the heartbeat only affects points that would otherwise go quiet: pushed points
    with static values (the OPC UA "offline" case), and points given a change filter. A polled
    point without a filter already publishes every reading.
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
  than an alarm's hysteresis. `ot-event` with `every = true` (trap occurrences) must not be
  combined with `on_change`; the config loader warns when a point has both
  `meta.event.every` and a `report` change filter.
- [A heartbeat could hide a stale source.] → For pushed points it is only sent while the link
  is `connected`, which the runtime's `check_subscription` probe keeps accurate. Polled points
  never publish without a fresh reading.
- [State lost on restart or reload means one extra publish per point.] → This is accepted and
  documented.
- [The C and Rust implementations could drift.] → Shared test vectors (D3), plus conformance
  cases run through the parity harness.
- [Trailing and debounced samples are sent late, with their original `ts`.] → Consumers
  already order by `ts`. The delay is at most the window plus one tick (200 ms).

## Migration Plan

- Opt-in, with no change for existing configs. Fresh installs get the 30-minute default
  heartbeat (D6).
- Users of `meta.on_change` and the other `meta` keys move to `report` by renaming the table.
  The migration guide and the RFC 0002 Cloud Fieldbus mapping (`noUpdateIfEqual` →
  `report.on_change`) are updated.
- Rollback: remove `report` from the config. The flow's settings are still available.

## Open Questions

- None. The heartbeat default was decided in D6, and percent of range was deferred.
