## ADDED Requirements

### Requirement: Reporting pass visits only points with a policy
The C runtime's reporting pass SHALL read the monotonic clock once per pass and SHALL visit only
points whose effective `report` policy is not passthrough. Points without a policy SHALL cost the
pass nothing. The policy's results (held, settled and heartbeat readings) SHALL be the same as
before this change.

#### Scenario: Large config without policies
- **WHEN** a C connector has 30,000 points and no `report` policy, and receives 4,000 changes/s
- **THEN** each pass reads the clock once and visits no point, and the connector's CPU follows the published rate instead of the point count

#### Scenario: Mixed config
- **WHEN** 10 of a device's 1000 points have `report = { min_interval = "5s" }`
- **THEN** the pass visits only those 10 points, and their held readings are published when their interval ends, as before

### Requirement: Polling loop visits only polled points
The C runtime's polling loop SHALL iterate only the points that are readable and not currently
delivered by push. It SHALL bring that set up to date whenever a point's push state changes: after
a subscribe, on a transport drop and on a reload. A point that loses its subscription in an outage
SHALL be polled from the next pass on, as before.

#### Scenario: Outage falls back to polling
- **WHEN** a device's subscribed points lose their subscription because the transport drops
- **THEN** from the next pass they are polled at their `poll_interval`, and once re-subscribed they leave the polling loop again

### Requirement: Main-loop wait follows the fastest pushed point
The C runtime SHALL wait between passes for no longer than the fastest effective
`sampling_interval` among its currently pushed points, clamped to between 10 ms and 200 ms. With
no pushed point it SHALL wait 200 ms. The wait SHALL be recomputed when the set of pushed points
changes. It SHALL apply to the MQTT wait, to the sleep without MQTT output and to the sleep while
the broker is unreachable. Activity on the MQTT socket SHALL still end the wait early.

#### Scenario: Fast subscription
- **WHEN** a C OPC UA connector's pushed points resolve to `sampling_interval = "100ms"`
- **THEN** the loop waits at most 100 ms between passes, and a value that arrives on the OPC UA socket is published within about 100 ms plus processing time

#### Scenario: Fastest-rate request
- **WHEN** a pushed point resolves to `sampling_interval = "0"`
- **THEN** the loop waits at most 10 ms between passes

#### Scenario: Polling-only connector
- **WHEN** a C Modbus connector has no pushed point
- **THEN** the loop waits 200 ms between passes, as before

### Requirement: Pushed start values are not lost
The C OPC UA connector SHALL buffer pushed values per device in a ring sized when the device
subscribes. The ring SHALL hold at least twice the device's armed monitored items and at least
256 entries. When it is full, it SHALL grow to twice its size, up to the larger of 65536 entries
and four times the armed items. Only past that bound, or when memory cannot be allocated, SHALL
the newest value be dropped. Every drop SHALL be counted and logged at `warn` with the device and
the bound. A grown ring SHALL keep its values in order.

#### Scenario: Start values of a large device
- **WHEN** a C connector subscribes a device with 20,000 points whose values do not change
- **THEN** a sample is published for each of the 20,000 points, and no drop is logged

#### Scenario: Small device stays small
- **WHEN** a device with 100 subscribed points subscribes
- **THEN** its ring holds 256 entries, not tens of thousands

#### Scenario: Burst beyond the ring
- **WHEN** a burst arrives that is larger than the ring but within the bound
- **THEN** the ring grows, every value of the burst is published in order, and no drop is logged
