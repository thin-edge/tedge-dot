## ADDED Requirements

### Requirement: Declared reporting policy
A connector configuration SHALL accept an optional `report` table in three places: on a point,
on a device, and in `[connector]`. The table's keys SHALL be `on_change` (bool), `deadband`
(a non-negative number, or a string `"<p>%"` with `p > 0`), `min_interval` (duration),
`max_interval` (duration) and `debounce` (duration). The effective policy of a point SHALL be
merged key by key: a key set on the point wins over the same key on its device, and a key set
on the device wins over the same key in `[connector]`. A point library SHALL be able to declare
`report` on its points. A site's point-level `report` keys SHALL win over the library's keys.
When a point has no effective policy, the SDK SHALL publish every reading, as it does today.
Both the Rust and C implementations SHALL apply the policy in the SDK runtime. A connector
module SHALL NOT filter samples on its own.

#### Scenario: Inheritance
- **WHEN** `[connector] report = { max_interval = "15m" }`, the device sets `report = { on_change = true }`, and one of its points sets `report = { deadband = 0.5 }`
- **THEN** that point's effective policy is `{ on_change = true, deadband = 0.5, max_interval = "15m" }`, and the device's other points use `{ on_change = true, max_interval = "15m" }`

#### Scenario: Invalid policy rejected
- **WHEN** a point sets `report = { min_interval = "10s", max_interval = "5s" }` or `deadband = "-1%"`
- **THEN** the configuration fails validation with an error that names the point and the key, and a reload keeps the running configuration

#### Scenario: No policy
- **WHEN** no `report` table applies to a point
- **THEN** every reading of that point is published

### Requirement: Change detection and deadband
With `on_change = true`, or with a `deadband` set, the SDK SHALL publish a numeric sample only
when the value after `transform` differs from the **last published** value of that point by at
least the threshold:

- An absolute deadband's threshold SHALL be the configured number.
- A `"<p>%"` deadband's threshold SHALL be `p/100 × |last published value|`. When the last
  published value is 0, any non-zero change SHALL pass.
- `on_change` without a deadband SHALL pass any difference greater than 1e-9.

For non-numeric values (bool, string, bytes) and raw-mode points, any change in the published
`value`, or in `raw` when there is no decoded value, SHALL pass, and `deadband` SHALL be
ignored.

#### Scenario: Absolute deadband
- **WHEN** a point has `deadband = 0.5`, the last published value is 100.0, and it then reads 100.4 and 100.6
- **THEN** 100.4 is not published and 100.6 is published

#### Scenario: Slow drift is reported
- **WHEN** a point has `deadband = 0.5`, the last published value is 100.0, and it then reads 100.3, 100.4, 100.5
- **THEN** 100.5 is published, because the difference is measured from the last published value and not from the previous reading

#### Scenario: Percent deadband
- **WHEN** a point has `deadband = "2%"`, the last published value is 200, and it then reads 203 and 204
- **THEN** 203 is not published and 204 is published

#### Scenario: Boolean on change
- **WHEN** a coil point has `on_change = true` and reads true, true, false
- **THEN** only the first true and the false are published

### Requirement: Always-published samples
Regardless of the policy, the SDK SHALL publish a sample:

- when it is the first sample of a point since the runtime started;
- when it is the first sample since a reload that changed that point's configuration or policy;
- when it is the first sample since the point's device reconnected;
- when its `quality` differs from the quality of the last published sample of that point.

A quality change SHALL NOT wait for `min_interval` or `debounce`.

#### Scenario: Quality change bypasses filters
- **WHEN** a point with `on_change = true` and `min_interval = "1m"` published a good 5.0, and 2 s later reads a bad sample
- **THEN** the bad sample is published at once

#### Scenario: First sample after restart
- **WHEN** the connector restarts and reads a value equal to the one it last published before the restart
- **THEN** that value is published

### Requirement: Rate limit with trailing publish
With `min_interval` set, the SDK SHALL publish at most one sample per point per window. A
sample that passes the other filters but arrives within `min_interval` of the last publish
SHALL be held as pending, and a newer pending sample SHALL replace an older one. When the
window ends, the SDK SHALL publish the pending sample if it still passes the change test
against the last published value, and it SHALL keep the sample's original `ts`. The delay
beyond the end of the window SHALL be at most one runtime tick.

#### Scenario: Last change in a burst is not lost
- **WHEN** a pushed point with `on_change = true` and `min_interval = "10s"` publishes 1 at t=0, and receives 2 at t=2 s and 3 at t=4 s with no further pushes
- **THEN** 3 is published at about t=10 s with its t=4 s timestamp, and 2 is never published

#### Scenario: Pending sample superseded back to the published value
- **WHEN** a point with `on_change = true` and `min_interval = "10s"` publishes 1 at t=0, receives 2 at t=2 s, and then 1 at t=4 s
- **THEN** nothing further is published when the window ends

### Requirement: Heartbeat
With `max_interval` set, the SDK SHALL make sure a point is published at least once per
`max_interval`, even when its value has not changed:

- For a polled point, the SDK SHALL publish the first reading taken at least `max_interval`
  after the last publish, whether or not it changed.
- For a pushed point with no reading in that time, the SDK SHALL re-publish the last published
  value with a current `ts`, but only while the device's link status is `connected`.
- The configuration SHALL be rejected when `max_interval` is not greater than `min_interval`.

#### Scenario: Flat polled signal
- **WHEN** a point polled every 10 s has `on_change = true, max_interval = "1m"` and always reads 42
- **THEN** 42 is published at start and then about once a minute

#### Scenario: Silent pushed signal
- **WHEN** a pushed point has `max_interval = "1m"`, its device link is `connected`, and no push arrives for 3 minutes
- **THEN** the last value is re-published about once a minute, each time with a new `ts`

#### Scenario: No heartbeat while disconnected
- **WHEN** the same pushed point's device link is `disconnected`
- **THEN** no heartbeat samples are published for it

### Requirement: Shipped default heartbeat
The connector configs installed by the packages, and the demo configs, SHALL set
`[connector] report = { max_interval = "30m" }`, with a comment explaining why. The SDK's own
default, when no `report` is configured anywhere, SHALL remain "no heartbeat and no
filtering".

#### Scenario: Fresh install keeps a static device available
- **WHEN** the packaged OPC UA config is used, and a subscribed node's value never changes for 2 hours while the link stays `connected`
- **THEN** the point is published at least every 30 minutes

#### Scenario: Config without a policy is unchanged
- **WHEN** a config sets no `report` anywhere
- **THEN** no heartbeat samples are published, and every reading is published

### Requirement: Debounce
With `debounce` set, the SDK SHALL publish a changed value only after the same value has been
seen continuously for `debounce`, measured on the runtime's monotonic clock. A value that
changes again during the period SHALL restart it. For a pushed point, the SDK SHALL publish the
settled value once the period ends, even when no further push arrives. `debounce` SHALL imply
change detection.

#### Scenario: Flapping value suppressed
- **WHEN** a point with `debounce = "2s"` last published 0, and then reads 1 at t=0, 0 at t=1 s, and 1 at t=1.5 s which stays unchanged
- **THEN** 1 is published at about t=3.5 s, and no 0 is published in between

### Requirement: Sequence numbers count published samples
The SDK SHALL increment a point's `seq` only when it publishes a sample of that point. A sample
held back by the policy SHALL NOT consume a sequence number, so a gap in `seq` still means a
published sample was lost.

#### Scenario: Suppressed samples leave no gap
- **WHEN** a point with `on_change = true` reads 1, 1, 1, 2
- **THEN** the published samples 1 and 2 carry consecutive `seq` values

### Requirement: Policy is discoverable
The capability descriptor SHALL publish `point_reports`: the effective `report` policy of every
point that has one, keyed by point id. The contract SHALL document the `report` field and its
semantics, and so SHALL the JSON config schema, the user-facing flows and connector docs, and
the migration guide. The `ot-measurement` flow's `on_change`, `deadband`, `min_interval` and
`debounce` settings SHALL be documented as deprecated in favour of `report`, and SHALL keep
their current behaviour.

#### Scenario: Descriptor shows the effective policy
- **WHEN** a device's point `temp` inherits `{ on_change = true, max_interval = "15m" }`
- **THEN** the retained capability descriptor contains `point_reports.temp` with those keys

#### Scenario: Legacy flow settings still work
- **WHEN** a point carries `meta = { on_change = true }` and no `report`
- **THEN** the SDK publishes every reading, and `ot-measurement` still suppresses unchanged measurements as before

### Requirement: Implementation parity
The Rust and C implementations SHALL produce the same published sequence for the same
configuration and timed input. Both implementations SHALL pass a shared set of test vectors,
and the layer-3 conformance suite SHALL cover `report` through the parity harness.

#### Scenario: Shared vectors
- **WHEN** the unit tests of both SDKs run the vectors in `doc/contract/test-vectors/report/`
- **THEN** both SDKs publish the expected samples at the expected times, with the expected `ts` and `seq`
