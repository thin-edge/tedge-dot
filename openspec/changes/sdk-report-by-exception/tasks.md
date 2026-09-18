## 1. Contract and schema

- [ ] 1.1 Add the `report` field to contract §3.1. Add a new "Reporting policy" section covering inheritance, semantics, always-published samples, and `seq`. Clarify that the §4 rule "driver MUST NOT apply thresholding" does not apply to the declared policy that the SDK applies.
- [ ] 1.2 Add a `report` definition to `config.schema.json` for point, device and `[connector]`, with validation of the `deadband` pattern and of the durations. Update the `meta` descriptions in the config and sample schemas so they no longer list `on_change`, `deadband` and the other policy keys as examples.
- [ ] 1.3 Add `point_reports` to the capability descriptor section (§7) and to its schema.
- [ ] 1.4 Write the shared test vectors in `doc/contract/test-vectors/report/*.json`, covering every scenario in the spec: deadband, drift, percent, bool, quality bypass, trailing publish, superseded pending sample, polled and pushed heartbeat, heartbeat while disconnected, debounce, and `seq`.

## 2. Rust SDK

- [ ] 2.1 Parse `report` in `config.rs` for connector, device and point. Merge it through point libraries in `library.rs`. Validate it, including `max_interval > min_interval`.
- [ ] 2.2 Add a pure `report` module: a `ReportState` per point with `offer(sample, now)` and `due(now, link_connected)`.
- [ ] 2.3 Wire it into `runtime.rs`. `publish_sample` calls `offer`, and `seq` is incremented only on publish. The tick calls `due`. Reset a point's state on reconnect and on a reload that changes the point.
- [ ] 2.4 Add `point_reports` to the descriptor.
- [ ] 2.5 Unit tests: run the shared vectors. Add a proptest that a published sequence never has `seq` gaps, and that no pending sample is lost after the last input once the window ends.

## 3. C SDK

- [ ] 3.1 Parse, merge and validate `report` in `config.c` and `include/tedge_dot/config.h` (connector, device, point, and libraries). Include it in the config fingerprint.
- [ ] 3.2 Add `sdk/src/report.c` with `tdot_report_offer` and `tdot_report_due`, keeping its state on `tdot_point_t`.
- [ ] 3.3 Wire it into `emit_sample`, `push_sink` and the main loop in `runtime.c`. Stamp `seq` only on publish, and reset the state on reconnect and on reload.
- [ ] 3.4 Add `point_reports` to the descriptor in `descriptor.c`.
- [ ] 3.5 Unit tests in `impl/c/tests`: run the shared vectors, and add config merge and validation tests.

## 4. Conformance and parity

- [ ] 4.1 Add layer-3 `ot-conformance` cases: on_change with a polled point, deadband, a flat signal with a heartbeat, `seq` continuity, and descriptor `point_reports`.
- [ ] 4.2 Add a push-path case using a subscribe-capable connector: the trailing publish, and the pushed heartbeat that stops while the link is disconnected.
- [ ] 4.3 Run the suites through the parity harness (`IMPL=rust` and `IMPL=c`) and record any gaps.

## 5. Flows

- [ ] 5.1 In `ot-measurement` (`flow.toml`, `params.toml.template` and the `main.js` header), mark `on_change`, `deadband`, `min_interval` and `debounce` as deprecated in favour of the point's `report`. Keep their behaviour.
- [ ] 5.2 Check that `ot-alarm`, `ot-event` and `ot-parameter-state` behave correctly on a filtered stream. Make the config loader warn when a point has both `meta.event.every` and a change filter in its `report`.

## 6. Documentation and demo

- [ ] 6.1 Write a user-facing "Reducing data volume (report by exception)" guide with examples for each connector, and link to it from `README.md`, `doc/README.md` and `flows/README.md`.
- [ ] 6.2 Add a short `report` subsection to each `doc/connectors/*-spec.md`, noting how it relates to protocol-side filtering (OPC UA subscriptions, traps).
- [ ] 6.3 Update the migration guide, `modbus-plugin-gap-analysis.md` and RFC 0002: `noUpdateIfEqual` and `transmitRate` now map to `report.on_change` and `report.min_interval`.
- [ ] 6.4 Update the demo configs (`demo/points.d/**`, `connectors/*/connector.toml`) to use `report` instead of `meta.on_change` and the other `meta` keys.
- [ ] 6.5 Add `[connector] report = { max_interval = "30m" }`, with an explanatory comment, to `packaging/config/*.toml`, `impl/c/packaging/config/profibus.toml` and `demo/config/*.toml`. Check that the manifest-parity script still passes.
- [ ] 6.6 Add a release-note entry to `packaging/release-notes.md`: the new `report` field, the deprecated flow settings, and a note that existing installs keep their `/etc` config and can opt in to the 30-minute heartbeat by adding one line.
