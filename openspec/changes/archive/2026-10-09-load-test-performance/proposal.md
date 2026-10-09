# Proposal

## Why

The OPC UA load test ([c8y-tedge-opcua-loadtest](https://github.com/Cumulocity-IoT/c8y-tedge-opcua-loadtest),
`docs/tedge-dot-changes.md` and `docs/tedge-dot-feedback.md`) drove one gateway to 30,000–150,000
tags and up to 140,000 changes/s. It needed a patched `tedge-dot-c` to get there, and found
limits in both builds:

- **C, CPU.** `report_pass()` reads the monotonic clock once per point on every pass of the main
  loop, including for points that have no `report` policy. At 30,000 points that was 87–89 % of
  the connector's CPU: a full core from 2,000 changes/s on. Reading the clock once per pass
  brought it down to a quarter of a core with the same deliveries. The loop over all points that
  remains still costs about 15 % at 30,000 points, and 85 % at 60,000 with a short tick.
- **C, latency.** The main loop waits up to 200 ms in `mosquitto_loop()` and does not watch the
  OPC UA socket meanwhile. Pushed values sit in the socket for up to 200 ms, so a
  `sampling_interval` below 200 ms is not met end to end. The CPU-bound `report_pass` hid this
  because it kept the loop busy. Fixing the CPU without fixing the wait trades CPU for latency.
- **C, lost start values.** Pushed values wait in a fixed ring of 256 per device
  (`UA_PUSH_QUEUE_LEN`). Every monitored item reports its current value when it is created. With
  more than about 255 points, most of the start values are dropped, and the drop only shows in
  the log. The load test used 32,768 slots (about 31 MiB per device, reserved even for small
  devices).
- **Both, lost changes.** Monitored items use a fixed `queueSize = 1`. A tag that changes more
  than once per sampling/publishing interval loses the changes in between at the server, and
  nothing reports it (about 10 % for fast tags; 0.02–0.04 % even for slow ones at high load). A
  queue of 16 lost nothing.
- **Rust, more than 1000 points per device fails.** All monitored items of a device go in one
  `CreateMonitoredItems` call, and async-opcua refuses a reply with more than 1000 entries. 5000
  points on one device gave `BadDecodingError`, a degraded link and no sample. Grouping points by
  node is also quadratic (a linear search per point). The C build creates one item per request,
  so subscribing 30,000 points takes 30,000 round trips.

## What Changes

- **C runtime: reporting pass costs only what has a policy.** The clock is read once per pass.
  The pass visits only points with a `report` policy, from a per-device index built at
  configure/reload. The polling loop also keeps a per-device list of polled points, so it no
  longer touches every point on every pass. The C main loop's CPU then depends on what is
  published, not on how many points are configured.
- **C runtime: adaptive main-loop wait.** The wait in `mosquitto_loop()` (and the sleep without
  MQTT) is the shortest of 200 ms and the fastest effective sampling interval of the connector's
  pushed points, with a floor of 10 ms. It is recomputed when subscriptions change. A connector
  without fast pushed points keeps the 200 ms tick.
- **C OPC UA: the push ring is sized per device and grows.** It starts at the number of armed
  monitored items plus headroom, and doubles when it is full, up to a bound. It is no longer a
  fixed array inside the device struct. The start values of every point fit. A drop past the
  bound is still logged, with the bound named.
- **Both OPC UA builds: configurable monitored-item queue, default 16.** A new `queue_size` key
  on `[connection]`, `device.protocol_address` and `point.address` (point, then device, then
  connection, then `16`). `discardOldest` stays `true`. **Behaviour change:** the default rises
  from 1 to 16, so a tag that changes several times per interval now delivers each change instead
  of the latest one. Set `queue_size = 1` for the old behaviour.
- **Both OPC UA builds: monitored items are created in batches.** At most 500 per
  `CreateMonitoredItems` request. Rust raises the client's array limit to match. A device with
  5000 or more subscribed points then subscribes in both builds. C goes from one request per item
  to one per batch.
- **Rust OPC UA: linear grouping.** Points are grouped into monitored items with a map, not a
  linear search per point.

Not in this change (in the feedback docs, tracked separately): per-device counters/metrics,
mapper and flow costs, mapper/bridge backpressure, batched sample output, Mosquitto limits,
server-side `DataChangeFilter` deadbands, and waiting on the OPC UA and MQTT sockets together.

## Capabilities

### New Capabilities
- `opcua-monitored-item-queue`: the `queue_size` setting (levels, precedence, validation,
  default 16) and what both builds request with it.
- `opcua-large-subscriptions`: devices with thousands of subscribed points subscribe in both
  builds (batched creation, the all-or-nothing rule across batches, the client array limit).
- `c-runtime-push-delivery`: how the C runtime delivers pushed samples: start values are not
  lost (growing per-device ring), a bounded wait derived from the fastest sampling interval, and
  a reporting/polling pass whose cost does not grow with points that have nothing to do.

### Modified Capabilities
- `push-sampling-interval`: the "OPC UA monitored-item sampling" requirement fixes
  `queueSize = 1`. It changes to the effective `queue_size`.

## Impact

- C: `impl/c/sdk/src/runtime.c` (`report_pass`, polling loop, `mqtt_service`, tick), SDK point
  and device structs for the per-device indices, `impl/c/connectors/opcua/connector_opcua.c`
  (ring, `queue_size` parsing, batched `UA_Client_MonitoredItems_createDataChanges`).
- Rust: `impl/rust/crates/connector-opcua/src/lib.rs` (`queue_size`, batched
  `create_monitored_items`, `max_array_length`, HashMap grouping) and its config parsing.
- Docs: `doc/connectors/opcua-connector-spec.md` §3.1/3.2/3.4/3.8, `impl/c/README.md` (tick note),
  `doc/reducing-data-volume.md` if it mentions queue size.
- Tests: unit tests in both builds. OPC UA e2e for >1000 points per device and for fast-changing
  tags with `queue_size`. C runtime tests for the tick and for the ring growth.
- Users: more samples from fast OPC UA tags by default. Lower CPU and latency for the C build at
  high point counts. Memory for the C push ring now scales with the device size instead of being
  fixed.
