# Design

## Context

The load test ran `tedge-dot-c` 0.0.11 with four patches (`docs/tedge-dot-changes.md` in the
load-test repo): ring 32768, item queue 16, clock once per device and pass, and a 10 ms tick.
Main has moved on since 0.0.11 (`sampling_interval`, `report` policies, `map`, config watch), but
none of those touched these four spots:

- `impl/c/sdk/src/runtime.c`:
  - `report_pass()` loops over every point of every device on each pass and calls `mono_ns()`
    before `report_state()` returns `NULL` for a passthrough point.
  - The polling loop also visits every point to skip the subscribed ones.
  - `mqtt_service()` waits `TICK_MS = 200` in `mosquitto_loop()`.
- `impl/c/connectors/opcua/connector_opcua.c`:
  - `ua_pending_t queue[UA_PUSH_QUEUE_LEN = 256]` is embedded in `ua_device_t`. A slot holds a
    full `tdot_sample_t`, about 1 KB.
  - `subscribe_device()` creates one monitored item per request, with `queueSize = 1`.
- `impl/rust/crates/connector-opcua/src/lib.rs`:
  - `subscribe` groups points with `monitored.iter_mut().find(..)`, which is O(n²).
  - It sends all items in one `create_monitored_items` with `queue_size: 1`.
  - The client keeps async-opcua's default `max_array_length` (1000).

The Rust runtime does not need the C runtime changes:

- Pushed samples arrive on `sample_rx` and are published straight away, without waiting for a
  tick.
- `Reporter` holds only the points that have a policy, and reads the clock once in `due()`.

The Rust build only takes the OPC UA connector changes.

## Goals / Non-Goals

**Goals:**
- C: CPU of the main loop proportional to work done, not to configured points.
- C: pushed OPC UA samples are handed over within about the fastest sampling interval (at least
  10 ms), not up to 200 ms late.
- C: no start value is lost because of the ring size, and small devices do not reserve tens of
  MiB.
- Both: monitored-item queue configurable, default 16, same value requested by both builds.
- Both: a device with ≥ 5000 subscribed points subscribes.
- Rust/C parity holds: the same file gives the same requested parameters (`IMPL` switch suites).

**Non-Goals:**
- Waiting on the OPC UA socket and the MQTT socket together. That needs a `wait_fd` connector
  hook, and open62541 has no public access to its event-loop descriptor. The load test tried it
  and dropped it.
- Metrics/counters, batched sample output, mapper/flow and bridge/Mosquitto findings.
- Server-side `DataChangeFilter` deadbands.
- Configurable ring bounds or batch size (no new keys beyond `queue_size`).

## Decisions

### D1. C reporting pass: one clock read, a per-device policy index
At configure (and on reload, which rebuilds the device array anyway) the runtime builds, per
device, an array of indices of the points whose effective `report` policy is not passthrough
(`tdot_report_is_passthrough`). `report_pass()` reads `mono_ns()` once per pass and walks only
those indices. Policy state is still allocated lazily by `report_state()`.

- *Alternative:* only hoist the clock, as in the load test. It removes the 87–89 %, but the loop
  over all points stays: 15 % at 30k, 85 % at 60k with a 10 ms tick. The index removes both, and
  it costs one `size_t` per policy point.
- *Why the pass-wide clock is safe:* the policy compares against intervals of milliseconds or
  more, and a pass takes microseconds per policy point. The Rust `Reporter::due` already uses one
  `now` for all points.

### D2. C polling loop: a per-device list of polled points, rebuilt when subscription state changes
`pt->subscribed` changes at runtime. `subscribe_device` sets it, and a transport drop clears it so
that the points get polled during an outage. Each device keeps an index array of points that are
readable and not subscribed. Every place that changes `subscribed` rebuilds it straight away:
`arm_subscriptions()` after `subscribe_device()`, and `clear_subscriptions()`, which runs on a
failed connect, on a transport drop and before re-arming, so also after a reload. Those events
are rare (at most one reconnect per second per device), so a dirty flag buys nothing. The
`any subscribed` scan before `drain_subscriptions` becomes a per-device count kept with the list.
The lists and the tick live in `sdk/src/schedule.c` (`tedge_dot/schedule.h`), so
`impl/c/tests/schedule.c` can check them against a full scan without a runtime. If a list cannot
be allocated, the loop walks every point and its own checks still decide what is done.

- *Alternative:* leave the loop. A skipped point is a branch, but at 60k points × 100 passes/s
  that is a full sweep of the point array (about 10 MB) per pass. That is memory traffic for
  nothing, and it would stop the short tick (D3) from paying off.

### D3. C adaptive wait: `clamp(fastest pushed sampling interval, 10 ms, 200 ms)`
The runtime keeps `rt->tick_ms`. It is recomputed whenever the set of subscribed points changes
(the same moments that rebuild the polled list): the minimum `sampling_interval_s` over points with
`subscribed == true`, clamped to [10, 200] ms, and 200 ms when nothing is pushed.
`mqtt_service()`, the no-MQTT `nanosleep` and the broker-down sleep use `tick_ms`. A
`sampling_interval = "0"` gives 10 ms.

- *Alternative: a fixed 10 ms tick* (load test). Every C connector, a Modbus one with `5s` polls
  too, would wake 100×/s. With D1/D2 the wakeup is cheap, but it is still wasted on idle
  gateways.
- *Alternative: wait on both sockets.* Rejected under Non-Goals.
- *Why the sampling interval, not the publishing interval:* they are the same number in the
  OPC UA module (publishing = fastest sampling), and `sampling_interval_s` is protocol-neutral.
  Other push modules (SNMP traps, CAN) benefit too.
- *Why only subscribed points:* polled points already have a schedule (`next_due`). A 200 ms
  tick is the existing documented polling granularity, and changing it is not this change. The
  `impl/c/README.md` note about "up to one tick (200 ms)" is updated to describe the adaptive
  tick.
- `mosquitto_loop` returns early on socket activity, so a busy connector keeps running passes
  back to back, as before.

### D4. C push ring: heap-allocated, sized at subscribe, grows by doubling up to a bound
`ua_device_t` holds a `ua_ring_t` (`connectors/opcua/push_ring.h`, header-only so it can be
unit-tested) in place of the fixed array. In `subscribe_device()`, *before* the items are
created, because creating them runs the client and can already deliver start values:

- `cap = next_pow2(max(256, 2 × wanted + 1))`, so all start values fit with room for the first
  changes.
- The ring is freed in `disconnect_device`. The SDK releases `ua_device_t` with a plain
  `free()`, always after a disconnect, so that is the only place it cannot leak. A reconnect
  sizes a new one. A flapping link reconnects at most once a second, so the extra allocation
  costs nothing.

In `on_data_change`, when the ring is full, it grows to `2 × cap`. This happens inside the
open62541 callback, on the same thread that drains, so no locking is needed. The existing entries
are re-laid in order from `tail`.

- The growth bound is `max(65536, 4 × armed)` slots. Past it, the newest change is dropped and
  the drop is logged with the bound, as today. 65536 slots is about 64 MiB, and it is only reached
  by a device whose backlog really is that large.
- Memory: a 1,000-point device starts at 2,048 slots (about 2 MiB) instead of the 31 MiB the load
  test reserved, and 30 small devices no longer cost about 1 GiB.
- *Alternative: a configurable ring size.* It adds a key for a knob that users cannot size
  correctly. The load test's own rule ("points + 3 × changes/s") is what growth gives without
  configuration.
- *Alternative: shrink the slot* (store the `UA_DataValue` and convert on drain). That is
  worthwhile, but it moves conversion out of the callback and touches the sample path. It is left
  as a follow-up.

### D5. `queue_size`: a module key at three levels, default 16
- **Where:** `[connection] queue_size`, `device.protocol_address.queue_size` and
  `point.address.queue_size`. Point wins over device, device over connection, and connection over
  the default of 16. This follows the existing `[connection]` → `protocol_address` override
  pattern in the OPC UA spec (§3.1/3.2). It is a module key, not a common `[[device]]` key, because
  only OPC UA has a server-side queue.
- **Validation:** an integer from 1 to 65535. A value of the wrong type or out of range is a
  configuration error naming the place and `queue_size`, with the same message in both builds.
  Unknown-key checking already covers misspellings where the module validates its keys.
- **Requested:** `queueSize = queue_size`, `discardOldest = true`. The server may revise it.
  A revised value that differs is logged at `info`, like the sampling interval (Rust: from
  `revised_queue_size`; C: from `mres.revisedQueueSize`).
- **Rust grouping (D6):** several points can share one monitored item, so the item takes the
  largest `queue_size` among them, the same way it takes the smallest interval. C creates one
  item per point and uses the point's value. With one point per node, which is the common case,
  both builds request the same values. With several points per node, C sends duplicate items with
  each point's own value. That difference already exists today (it applies to the sampling
  interval too) but is not documented. This change documents it in the OPC UA spec §3.8.
- **Default 16** (user decision): fast tags deliver every change, as the load test measured. The
  cost is more notifications per publish for tags that change faster than the publishing
  interval. That is intended, and `report.min_interval` / `queue_size = 1` limit it. It is called
  out as a behaviour change in the release notes and the spec doc.

### D6. Rust: HashMap grouping and batched creation
- Grouping uses `HashMap<(NodeId, Option<u32>), usize>` (key → index into `monitored`), which is
  O(n). `by_node` is unchanged.
- `create_monitored_items` is called per chunk of `MONITORED_ITEMS_PER_REQUEST = 500`, and the
  results are concatenated in order. The client is built with `max_array_length` raised (for
  example to `max(65535, default)`), so other large replies (reads, browse) do not hit 1000 either.
  The batch size alone keeps create replies under 1000.
- **All-or-nothing across batches.** If any batch errors or rejects an item, the whole
  subscription is deleted. That also drops the items of the earlier batches. The error is the
  same as today. Batching does not change which outcome a device gets.
- **Why 500:** under the 1000 decode limit, and within what common servers accept per call
  (`MaxMonitoredItemsPerCall` is often 1000 or unlimited). Reading the server's
  `OperationLimits` is deferred (Open Questions).

### D7. C: batched creation with `UA_Client_MonitoredItems_createDataChanges`
- C collects the points to arm, then calls `UA_Client_MonitoredItems_createDataChanges` per chunk
  of 500, with parallel arrays of contexts and callbacks. Each result maps back to its point:
  `subscribed = true` on good, and the per-point revised-interval and revised-queue logs.
- The C build's existing semantics stay as they are: a refused item stays polled, and the
  subscription is torn down only when none was armed. This differs from Rust's all-or-nothing.
  The difference exists today but is undocumented, so this change documents it. Batching does
  not touch it.
- A failed batch (service fault) leaves its points polled, like individually refused items.
- **Why:** 30,000 single requests is 30,000 round trips at every (re)connect. Batching makes
  subscribe time and server load proportional to batches. The open62541 client has no
  1000-element decode cap, so for C this is a speed change, not a fix.

## Risks / Trade-offs

- **[Default queue 16 raises notification volume for fast tags]** → Documented as a behaviour
  change. `queue_size = 1` restores the old behaviour. Report policies still apply downstream.
  The Rust/C parity suites assert the same requested value.
- **[Adaptive tick wakes up to 100×/s for configs with fast pushed points]** → Only when the user
  asked for sampling faster than 200 ms. D1/D2 make each wakeup cost what is published. It is
  measured with the load test's 30k/60k configs before merge.
- **[Ring growth in a callback can fail (`realloc` NULL)]** → Treated as full: the change is
  dropped and counted, never a crash. The old ring stays valid.
- **[Ring memory now bounded by backlog, not by a constant]** → The bound (`max(65536, 4 ×
  armed)`) caps it. The drop log names the bound. A device whose ring keeps hitting the bound is a
  device the main loop cannot keep up with, and a larger ring would not fix that.
- **[Batching changes subscribe timing; a slow server may now exceed `operation_timeout` on one
  large batch instead of many small calls]** → Each batch is a separate service call with the
  usual request timeout. The `subscribe` call overall is bounded by the runtime as today. A
  connect of 30k points gets faster, not slower.
- **[Policy/polled indices go stale]** → Rebuilt at every place that changes the underlying
  state. A debug-build assert compares the index against a full scan in the C unit tests.

## Migration Plan

- No config migration. Existing files keep working. The only visible change is `queue_size`
  defaulting to 16.
- Release notes: "OPC UA monitored items now queue up to 16 values by default; set
  `queue_size = 1` in `[connection]` for the previous behaviour."
- Rollback: revert. No persisted state is involved.

## Open Questions

- Should the batch size follow the server's `MaxMonitoredItemsPerCall` (read from
  `Server_ServerCapabilities_OperationLimits` at connect) when that is lower than 500? This
  change assumes it is not needed and leaves it as a follow-up if a server refuses 500.
- Re-measure with the load test (30k points at 20k/s, 60k points at 50–95k/s) on both builds
  before the release. These target numbers come from the patched build, not from this design.
