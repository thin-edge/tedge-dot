## 1. C runtime: reporting pass and polling loop (D1, D2)

- [x] 1.1 Add per-device index arrays to the C device state: points with a non-passthrough `report` policy (built at configure/reload) and polled points (readable, not `subscribed`) and a pushed-point count (`sdk/src/schedule.c`)
- [x] 1.2 Rewrite `report_pass()` in `impl/c/sdk/src/runtime.c` to read `mono_ns()` once per pass and walk only the policy index
- [x] 1.3 Make the polling loop iterate the polled-points index; rebuild it whenever `subscribed` changes (`arm_subscriptions`, `clear_subscriptions`); replace the `any subscribed` scan with the pushed count
- [x] 1.4 Free the indices on reload/shutdown; add a debug-build check in `impl/c/tests` that the indices match a full scan after subscribe, transport drop and reload
- [x] 1.5 Confirm `impl/c/tests/report.c` and the report-by-exception e2e still pass unchanged (same held/settled/heartbeat results)

## 2. C runtime: adaptive main-loop wait (D3)

- [x] 2.1 Add `rt->tick_ms`, recomputed whenever pushed points change, as the minimum `sampling_interval_s` of `subscribed` points clamped to [10, 200] ms (200 ms with none)
- [x] 2.2 Use `tick_ms` in `mqtt_service()` (`mosquitto_loop`, broker-down sleep) and in the no-MQTT `nanosleep`; leave the shutdown flush and other `TICK_MS` uses that are not the pass wait unchanged
- [x] 2.3 Unit-test the tick computation (no pushed points, 100 ms, `0` → 10 ms, 5 s → 200 ms, outage clearing `subscribed`)
- [x] 2.4 Update the "up to one tick (200 ms)" note in `impl/c/README.md` to describe the adaptive tick

## 3. C OPC UA: growing push ring (D4)

- [x] 3.1 Replace the fixed `queue[UA_PUSH_QUEUE_LEN]` in `ua_device_t` with a heap ring (`push_ring.h`); size it in `subscribe_device()` before the items are created to `next_pow2(max(256, 2 × wanted + 1))`; free it in `disconnect_device`
- [x] 3.2 In `on_data_change`, grow to `2 × cap` (re-laying entries in order from `tail`) when full, up to `max(65536, 4 × armed)`; on the bound or a failed allocation drop the newest value and count it
- [x] 3.3 Update the drop warning in `drain_subscriptions` to name the bound; free the ring in `disconnect_device`/destroy
- [x] 3.4 Unit-test ring growth, ordering across a wrap and a grow, the bound, and allocation failure (in `impl/c/tests/opcua_sampling.c` or a new test)

## 4. OPC UA `queue_size` in both builds (D5)

- [x] 4.1 Rust: parse `queue_size` from `[connection]`, `protocol_address` and point `address` (integer 1..=65535, error naming the place and key), resolving point → device → connection → 16
- [x] 4.2 C: the same parsing and resolution in `connector_opcua.c`, with the same error messages as Rust
- [x] 4.3 Rust: request `queue_size` per monitored item (largest among points sharing an item), `discard_oldest: true`; log an `info` line when `revised_queue_size` differs
- [x] 4.4 C: set `requestedParameters.queueSize` from the point's effective value; log an `info` line when `revisedQueueSize` differs
- [x] 4.5 Add `queue_size` to the config-validation/golden tests of both builds (valid at each level, 0, 65536, wrong type) and check that both builds give identical messages
- [x] 4.6 Add an OPC UA e2e case: a sim node that changes several times per publishing interval delivers every change with the default, and only the latest with `queue_size = 1`, for both `IMPL` values

## 5. Batched monitored-item creation (D6, D7)

- [x] 5.1 Rust: replace `monitored.iter_mut().find(..)` grouping with a `HashMap` key → index
- [x] 5.2 Rust: call `create_monitored_items` in chunks of 500 and concatenate the results; on any batch error or rejected item delete the subscription (all-or-nothing as today)
- [x] 5.3 Rust: raise the client's `max_array_length` (ClientBuilder) to at least 65535
- [x] 5.4 C: collect the points to arm and create them with `UA_Client_MonitoredItems_createDataChanges` in chunks of 500, mapping results back to points (`subscribed`, revised-interval and revised-queue logs); a failed batch leaves its points polled
- [x] 5.5 Add an OPC UA e2e case with ≥ 5000 subscribed points on one device (the sim must serve them) that asserts `connected` and a published change, for both `IMPL` values
- [x] 5.6 Add a unit test of the Rust chunking (order, batch sizes, stop at a failed batch) and an in-process integration test subscribing 5000 points

## 6. Docs

- [x] 6.1 `doc/connectors/opcua-connector-spec.md`: `queue_size` in §3.1/3.2/3.4, and in §3.8 replace "`queueSize = 1`" with the effective queue size and its default of 16; describe batched creation and the Rust/C difference on refused items and shared nodes
- [x] 6.2 Note the default change (1 → 16) and how to restore it (spec §3.8; the repo has no changelog, so also in the PR description for the release notes)
- [x] 6.3 Mention `queue_size` next to `report.min_interval` in `doc/reducing-data-volume.md` if it covers OPC UA subscriptions

## 7. Verification

- [x] 7.1 `just build` and the full unit-test suites of both builds; the OPC UA e2e and conformance suites with `IMPL=rust` and `IMPL=c`
- [ ] 7.2 Re-run the load test's 30k-points/20k-changes/s and 60k-points/50k–95k-changes/s scenarios against both builds and record CPU, memory and delay against the patched-build numbers in `docs/loadtest-results.md` of the load-test repo
