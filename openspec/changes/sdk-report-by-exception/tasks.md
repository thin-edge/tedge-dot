## 1. Contract and schema

- [x] 1.1 Add the `report` field to contract §3.1. Add a new "Reporting policy" section covering precedence, disabling with `"0"`, semantics, always-published samples, resets, heartbeat by fresh read, `seq`, and the CLI exemption. Clarify that the §4 rule "driver MUST NOT apply thresholding" does not apply to the declared policy that the SDK applies.
- [x] 1.2 Add a `report` definition to `config.schema.json` for point, device and `[connector]`, and to `point-library.schema.json`, with validation of the `deadband` pattern and of the durations. Update the `meta` descriptions in the config and sample schemas so they no longer list `on_change`, `deadband` and the other policy keys as examples.
- [x] 1.3 Document the descriptor's `reports` object (`default`, `devices`, `points`) in §7 and in `status.schema.json`.
- [x] 1.4 Write the shared test vectors in `doc/contract/test-vectors/report/*.json` (NaN encoded as the JSON `null` the envelope carries), covering every scenario in the spec: deadband, drift, percent, bool, NaN, int64 as a string, quality bypass, the stale pending sample, trailing publish, the superseded pending sample, polled heartbeat, pushed heartbeat read (ok, failed, unsupported), debounce with and without a deadband, resets, and `seq`.

## 2. Rust SDK

- [x] 2.1 Parse `report` in `config.rs` for connector, device and point. Add `report` to `DEEP_MERGED_KEYS` in `library.rs`. Validate each table on its own, including `max_interval > min_interval`. For a conflict that arises only through inheritance, raise the effective heartbeat and log a warning.
- [x] 2.2 Add a pure `report` module: a `ReportState` per point with `offer(sample, now)` and `due(now)`, which returns the ready pending or debounced samples and the heartbeat reads that are due. Add a reset for a point, for a device, and for everything.
- [x] 2.3 Wire it into `runtime.rs`: `publish_sample` (poll and push) and `print_sample` / `run_stdout_until` call `offer`, and `seq` is stamped only on publish. The tick calls `due` and runs the heartbeat reads through `bounded(read_points)`, grouped per device. Treat `Unsupported`, or `Ok` without a sample for the point, as "no data": no heartbeat, and no retry until a reset. Turn any other `Err` or a timeout into a published bad sample, fed to `note_poll` and the reconnect logic like a failed poll. Reset on reload or management command, on device reconnect, and in `restore_mqtt_session`.
- [x] 2.4 Add `reports` to the descriptor (`descriptor.rs`).
- [x] 2.5 Make the config loader warn when a point combines `meta.event.every` with a change filter or debounce.
- [x] 2.6 Unit tests: run the shared vectors. Add a proptest that published `seq` values are gap-free when the broker is online, that a quality change is never preceded in the output by an older pending sample, and that no pending sample is lost once its window ends.

## 3. C SDK

- [x] 3.0 Add `TDOT_READ_NO_DATA` to the `read_point` contract in `include/tedge_dot/connector.h`, and handle it in the poll path (publish nothing, link unaffected). Make `connector_snmp.c` return it for trap and varbind points, and `connector_canbus.c` return it when no new frame has arrived since the point's previous read. Check that the canbus and SNMP e2e suites still pass (with C canbus, fewer samples are expected when frames are periodic).
- [x] 3.1 Parse, merge and validate `report` in `config.c` and `include/tedge_dot/config.h` (connector, device and point, with libraries deep-merged like `meta` and `transform`), using the same inheritance-conflict rule as 2.1.
- [x] 3.2 Add `sdk/src/report.c` with `tdot_report_offer`, `tdot_report_due` and `tdot_report_reset_*`, keeping its state on `tdot_point_t`.
- [x] 3.3 Wire it into `emit_sample` (poll, `push_sink` and stdout mode) and into the main loop: trailing publishes, and heartbeat reads through `read_point` bounded by the operation timeout. `-1` publishes a bad sample and goes through the normal transport-down handling. `TDOT_READ_NO_DATA` marks the point unreadable until a reset. Stamp `seq` only on publish. Reset in `commit_config`, on device reconnect, and in the connect callback when a session is resumed.
- [x] 3.4 Add `reports` to the descriptor in `descriptor.c`.
- [x] 3.5 Add the same `meta.event.every` warning as 2.5.
- [x] 3.6 Unit tests in `impl/c/tests`: run the shared vectors, and add config merge, validation and inheritance-conflict tests.

## 4. Conformance, parity and e2e

- [x] 4.1 Add layer-3 `ot-conformance` cases: on_change with a polled point, deadband, a flat signal with a heartbeat, `seq` continuity with suppression, and the descriptor's `reports`. Keep `report` off the point that B2-seq (`layer3.rs`) samples, so it still sees at least 3 samples in its window.
- [ ] 4.2 Add push-path cases with the OPC UA simulator: the trailing publish, and the heartbeat read of a static node. Add an SNMP case: a trap point inheriting `max_interval` produces nothing without a trap.
- [ ] 4.3 Run the suites through the parity harness (`IMPL=rust` and `IMPL=c`) and record any gaps.
- [ ] 4.4 Update the e2e suites and configs that use the flow's `meta` keys: `connectors/opcua/tests/opcua_e2e.robot` (the `meta.on_change` assertion), `connectors/opcua/conformance/connector*.toml`, and `connectors/modbus/conformance/connector.toml`. Add an e2e case in which a static OPC UA node keeps the device available through the heartbeat.

## 5. Flows

- [x] 5.1 In `ot-measurement` (`flow.toml`, `params.toml.template` and the `main.js` header), mark `on_change`, `deadband`, `min_interval` and `debounce` as deprecated in favour of the point's `report`. Keep their behaviour and their tests in `flows/test-flows.sh`, and add a test note saying they are legacy.
- [x] 5.2 Check that `ot-alarm`, `ot-event` and `ot-parameter-state` behave correctly on a filtered stream, and document the rule that a deadband should be smaller than the alarm's hysteresis.

## 6. Documentation, configs and release

- [x] 6.1 Write a user-facing "Reducing data volume (report by exception)" guide with examples for each connector, and link to it from `README.md`, `doc/README.md` and `flows/README.md`.
- [x] 6.2 Add a short `report` subsection to each `doc/connectors/*-spec.md`, stating whether the connector's points support a heartbeat read (OPC UA, Modbus, CANopen SDO and PROFIBUS do; CAN frames and SNMP traps do not).
- [x] 6.3 Update the migration guide, `modbus-plugin-gap-analysis.md` and RFC 0002: `noUpdateIfEqual` and `transmitRate` now map to `report.on_change` and `report.min_interval`.
- [x] 6.4 Update the demo point libraries (`demo/points.d/**`) and `connectors/*/connector.toml` to use `report` instead of `meta.on_change` and the other `meta` keys.
- [x] 6.5 Add an active `[connector] report = { max_interval = "30m" }`, with an explanatory comment, to `packaging/config/{modbus,opcua,canopen}.toml`, `impl/c/packaging/config/profibus.toml` and the matching `demo/config/*.toml`. Add it as a commented-out example only to the canbus and snmp configs.
- [x] 6.6 Add a release-note entry to `packaging/release-notes.md`: the new `report` field, the deprecated flow settings, and a note that existing installs keep their `/etc` config and can opt in to the 30-minute heartbeat by adding one line.
