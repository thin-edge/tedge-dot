## ADDED Requirements

### Requirement: Declared reporting policy
A connector configuration SHALL accept an optional `report` table in three places: on a point,
on a device, and in `[connector]`. The table's keys SHALL be:

- `on_change` (bool);
- `deadband` (a non-negative number, or a string `"<p>%"` with `p > 0`);
- `min_interval`, `max_interval` and `debounce` (durations, where `"0"` means disabled).

The SDK SHALL merge a point's effective policy key by key, with each level winning over the
ones before it:

1. `[connector]`
2. `[[device]]`
3. the point as declared in a point library
4. the point as declared in the site's config

A more specific level SHALL be able to disable an inherited setting with `"0"`, or with
`on_change = false` and `deadband = 0`. When a point has no effective policy, the SDK SHALL
publish every reading, as it does today. Both the Rust and C implementations SHALL apply the
policy in the SDK runtime, in the MQTT and the stdout output modes. A connector module SHALL
NOT filter samples on its own. The CLI `read` command SHALL NOT apply the policy.

#### Scenario: Inheritance
- **WHEN** `[connector] report = { max_interval = "15m" }`, the device sets `report = { on_change = true }`, and one of its points sets `report = { deadband = 0.5 }`
- **THEN** that point's effective policy is `{ on_change = true, deadband = 0.5, max_interval = "15m" }`, and the device's other points use `{ on_change = true, max_interval = "15m" }`

#### Scenario: Library point merges with the site override
- **WHEN** a library point declares `report = { deadband = 1.0, min_interval = "10s" }` and the site's config sets `report = { deadband = 0.2 }` on that point
- **THEN** the effective policy is `{ deadband = 0.2, min_interval = "10s" }`

#### Scenario: Inherited setting disabled
- **WHEN** `[connector] report = { max_interval = "30m" }` and a point sets `report = { max_interval = "0" }`
- **THEN** that point has no heartbeat

#### Scenario: Invalid policy rejected
- **WHEN** a single `report` table sets `{ min_interval = "10s", max_interval = "5s" }`, or sets `deadband = "-1%"`
- **THEN** the configuration fails validation with an error that names the point (or device) and the key, and a reload keeps the running configuration

#### Scenario: Conflict only through inheritance is tolerated
- **WHEN** `[connector] report = { max_interval = "30m" }` and a point sets `report = { min_interval = "1h" }`
- **THEN** the configuration is accepted, the point's effective heartbeat is 2 h, and a warning names the point

#### Scenario: No policy
- **WHEN** no `report` table applies to a point
- **THEN** every reading of that point is published

### Requirement: Change detection and deadband
With `on_change = true`, or with a `deadband` set, the SDK SHALL publish a sample only when it
differs from the **last published** sample of that point.

For samples with `value_repr = "number"`, the SDK SHALL compare the value after `transform`,
and a sample SHALL count as changed when it differs by at least the threshold:

- An absolute deadband's threshold SHALL be the configured number.
- A `"<p>%"` deadband's threshold SHALL be `p/100 × |last published value|`. When the last
  published value is 0, any non-zero change SHALL pass.
- `on_change` without a deadband SHALL pass any difference greater than 1e-9.
- NaN SHALL be equal only to NaN.

All other samples SHALL be compared by exact equality of `(value_repr, value)`, or of `raw`
when there is no decoded value, and `deadband` SHALL be ignored. This covers bool, string,
bytes, raw mode, and 64-bit integers carried as strings.

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

#### Scenario: NaN does not block the point
- **WHEN** a point with `on_change = true` publishes NaN, then reads NaN, then 5.0
- **THEN** the second NaN is not published and 5.0 is published

### Requirement: Always-published samples and resets
The SDK SHALL publish a sample regardless of every other rule, including `min_interval` and
`debounce`, when either of these holds:

- the sample is the point's first since the runtime started, or since its policy state was
  reset;
- the sample's `quality` differs from the quality of the point's last published sample.

Publishing a quality change SHALL discard the point's pending sample and its debounce
candidate. The SDK SHALL reset the policy state of every affected point on each of these
events:

- an applied reload or configuration management command (all points);
- a device reconnect (that device's points);
- an MQTT session restore after a broker reconnect (all points).

#### Scenario: Quality change bypasses filters
- **WHEN** a point with `on_change = true` and `min_interval = "1m"` published a good 5.0, and 2 s later reads a bad sample
- **THEN** the bad sample is published at once

#### Scenario: Stale pending sample dropped on quality change
- **WHEN** a point with `min_interval = "10s"` published a good 1 at t=0, holds a good 2 as pending from t=2 s, and then reads a bad sample at t=3 s
- **THEN** the bad sample is published at t=3 s, and the good 2 is never published

#### Scenario: First sample after restart
- **WHEN** the connector restarts and reads a value equal to the one it last published before the restart
- **THEN** that value is published

#### Scenario: Broker reconnect
- **WHEN** a point with `on_change = true` reads a changed value while the broker is unreachable (so the sample is lost), and then the broker comes back and the value stays unchanged
- **THEN** the first reading after the MQTT session restore is published

### Requirement: Rate limit with trailing publish
With `min_interval` set, the SDK SHALL publish at most one sample per point per window. A
sample that passes the other filters but arrives within `min_interval` of the last publish
SHALL be held as pending, and a newer pending sample SHALL replace an older one. When the
window ends, the SDK SHALL publish the pending sample on the first pass of the main loop, if
it still passes the change test against the last published value, and it SHALL keep the
sample's original `ts`.

#### Scenario: Last change in a burst is not lost
- **WHEN** a pushed point with `on_change = true` and `min_interval = "10s"` publishes 1 at t=0, and receives 2 at t=2 s and 3 at t=4 s with no further pushes
- **THEN** 3 is published shortly after t=10 s with its t=4 s timestamp, and 2 is never published

#### Scenario: Pending sample superseded back to the published value
- **WHEN** a point with `on_change = true` and `min_interval = "10s"` publishes 1 at t=0, receives 2 at t=2 s, and then 1 at t=4 s
- **THEN** nothing further is published when the window ends

### Requirement: Heartbeat
With `max_interval` set, the SDK SHALL publish a point's first **fresh reading** taken at least
`max_interval` after its last publish, even when the value has not changed. The SDK SHALL
never re-publish an earlier sample as a heartbeat.

- For a polled point, the fresh reading SHALL be its next scheduled read.
- For a pushed point, the SDK SHALL request an on-demand read of the point through the
  module's ordinary read path, bounded by the operation timeout. A point SHALL get at most
  one such read per `max_interval`.
  - If the read fails or times out, the SDK SHALL publish a bad-quality sample for the point,
    and the failure SHALL affect the link status and reconnects exactly as a failed poll does.
  - If the module reports that it has nothing to read, the point SHALL get no heartbeat, and
    the SDK SHALL NOT retry it before the point's next reset. In Rust that is `Unsupported`,
    or `Ok` without a sample for the point. In C it is `TDOT_READ_NO_DATA`.
- A C module's `read_point` SHALL return `TDOT_READ_NO_DATA` for a point that cannot be read
  on demand (SNMP trap and varbind points), or that has no new data since its previous read
  (a CAN frame that has not been received again). On any read, the runtime SHALL publish
  nothing for that result.

#### Scenario: Flat polled signal
- **WHEN** a point polled every 10 s has `on_change = true, max_interval = "1m"` and always reads 42
- **THEN** 42 is published at start, and after that at intervals of at least 1 minute and at most 1 minute plus one poll interval

#### Scenario: Silent pushed signal
- **WHEN** a pushed OPC UA point has `max_interval = "1m"` and no push arrives for 3 minutes
- **THEN** the runtime reads the node about once a minute and publishes each reading with its own fresh `ts`

#### Scenario: Unreachable pushed source
- **WHEN** that heartbeat read fails because the server is gone
- **THEN** a bad-quality sample is published, and no good-quality sample is published

#### Scenario: Cached CAN frame is not re-published as fresh
- **WHEN** a C canbus point polled every 1 s has `max_interval = "1m"` and its frame stops arriving
- **THEN** no further sample of that point is published after the last received frame

#### Scenario: Unreadable point gets no heartbeat
- **WHEN** an SNMP trap point or a CAN frame point inherits `max_interval = "30m"` and no trap or frame arrives for 2 hours
- **THEN** nothing is published for that point

### Requirement: Shipped default heartbeat
Each packaged and demo config SHALL set `[connector] report = { max_interval = "30m" }`, with
a comment explaining why, for the modbus, opcua, canopen and profibus protocols. The canbus and snmp configs SHALL show the setting only as a commented-out example. The
SDK's own default, when no `report` is configured anywhere, SHALL remain "no heartbeat and no
filtering".

#### Scenario: Fresh install keeps a static device available
- **WHEN** the packaged OPC UA config is used, and a subscribed node's value never changes for 2 hours while the server stays reachable
- **THEN** the point is published at least every 30 minutes plus one loop iteration

#### Scenario: Config without a policy is unchanged
- **WHEN** a config sets no `report` anywhere
- **THEN** no heartbeat reads are made, and every reading is published

### Requirement: Debounce
With `debounce` set, the SDK SHALL publish a changed value only after it has stayed the same
for `debounce`, measured on the runtime's monotonic clock:

- "The same" SHALL mean within the deadband when one is set, and within 1e-9 (or exactly
  equal, for non-numeric values) otherwise.
- A reading that is not the same as the current candidate SHALL replace it and restart the
  period. A reading that is not a change from the last published value SHALL instead clear
  the candidate.
- The published sample SHALL be the most recent reading of the stable run.
- For a pushed point, the SDK SHALL publish the settled value once the period ends, even when
  no further push arrives.
- `debounce` SHALL imply change detection.

#### Scenario: Flapping value suppressed
- **WHEN** a point with `debounce = "2s"` last published 0, and then reads 1 at t=0, 0 at t=1 s, and 1 at t=1.5 s which stays unchanged
- **THEN** 1 is published shortly after t=3.5 s, and no 0 is published in between

#### Scenario: Noisy value settles within the deadband
- **WHEN** a point with `debounce = "2s", deadband = 0.5` last published 10.0, and reads 12.0, 12.1, 11.9 and 12.05 at t=0, 0.5, 1.0 and 1.5 s with no further reading
- **THEN** 12.05 is published shortly after t=2 s, and it carries the `ts` of the 12.05 reading

### Requirement: Sequence numbers count published samples
The SDK SHALL stamp a point's `seq` only when the policy decides to publish a sample of that
point. A sample held back by the policy SHALL NOT consume a sequence number, so a gap in `seq`
still means a published sample was lost.

#### Scenario: Suppressed samples leave no gap
- **WHEN** a point with `on_change = true` reads 1, 1, 1, 2
- **THEN** the published samples 1 and 2 carry consecutive `seq` values

### Requirement: Policy is discoverable
The capability descriptor SHALL publish a `reports` object with three parts:

- `default`: the policy declared in `[connector]`;
- `devices`: a list of `{device, report}` entries, one for each device that declares a policy;
- `points`: a list of `{device, point, report}` entries with the effective policy of each point
  whose policy differs from its device's effective policy.

The contract SHALL document the `report` field and its semantics, and so SHALL the JSON config
and status schemas, the user-facing flows and connector docs, and the migration guide. The
`ot-measurement` flow's `on_change`, `deadband`, `min_interval` and `debounce` settings SHALL be
documented as deprecated in favour of `report`, and SHALL keep their current behaviour.

#### Scenario: Descriptor shows the policy without repeating defaults
- **WHEN** `[connector] report = { max_interval = "15m" }`, device `plc-1` declares `report = { on_change = true }`, and only its point `temp` sets `deadband = 0.5`
- **THEN** the retained descriptor carries `reports.default = { max_interval = "15m" }`, one entry in `reports.devices` for `plc-1`, and one entry in `reports.points` for (`plc-1`, `temp`) with `{ on_change = true, deadband = 0.5, max_interval = "15m" }`

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
