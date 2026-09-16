# TODO

## In flight / next

* [ ] Ship profibus in the `tedge-dot-rs` package: the `profibus` cargo feature is excluded
      from the goreleaser builds because its serial dependency (`serialport` via `profirust`)
      has a native libudev build script that does not cross-compile with cargo-zigbuild.
      Options: disable the libudev feature upstream, vendor a libudev stub per target, or build
      the Linux packages natively per architecture. (`tedge-dot-c` already ships PROFIBUS, over
      `tcp://` only.)

* [ ] Close the remaining C parity gaps listed in `impl/c/README.md`. Each has a capability
      name already wired into the test tagging (`C_MISSING_CAPABILITIES` in the justfile), so
      implementing one means removing it from that list and adding the test that was waiting
      for it:
      - `opcua-security` — open62541 supports `Basic256Sha256` and friends; needs config +
        certificate plumbing, and a secured endpoint in the e2e stack to test against.
      - `canbus-fd` — classic frames only today; the Rust build has a `canbus-fd` feature.
      - `profibus-serial` — the C module speaks `tcp://` only (no serial PHY, no FDL token
        timing), so it cannot yet drive a multi-master RS-485 bus.
      - `snmpv3-sha2` — the C build links net-snmp's own crypto (`--with-openssl=internal`), which
        covers MD5/SHA-1 authentication and DES/AES-128 privacy; SHA-224…512 and AES-192/256 need
        a real OpenSSL, which would cost the small, dependency-free package. A device configured
        with one of them loads but stays `disconnected`, naming the capability.
      Also: CAN bus push delivery (the C module renders the push-based bus as drain-into-cache
      polling — same samples, worse latency, so it is not tagged), and the 255/256-byte cap on
      string/raw values (`tdot_value_t.str`, `TDOT_RAW_MAX`).

* [ ] SNMP connector follow-ups (`doc/connectors/snmp-connector-spec.md`; polling, SET writes,
      v1/v2c/v3 traps and informs shipped):
      - **File the `snmp2` issues upstream** and drop the patches as they are released: six drafts
        wait in `doc/upstream/snmp2-*.md` and the patched copy is `impl/rust/vendor/snmp2`
        (with its `TEDGE-DOT-PATCH.md`), the same arrangement as `vendor/async-opcua-crypto`.
      - **Table walks** (GETBULK over a column, one sample per row): deliberately deferred — rows
        exist only at runtime while the contract's point list is static (link status `points`,
        the capability descriptor, "one sample per point per read"), so it needs an additive RFC.
        Sketch: one point per column (`address = { column = "…", label = "…" }`) publishing a
        sample per row told apart by `addr.index`, with `ot-measurement`/`ot-alarm` naming the
        series per row.
      - MIB name resolution (`sysUpTime.0` instead of the numeric OID): `snmp2` has a `mibs`
        feature needing `libnetsnmp`, the C build links net-snmp already — but the packages would
        have to ship MIB files.
      - A catch-all for notifications from unconfigured senders (today dropped at debug level),
        which would let a discovery flow onboard equipment with `define-device`.
      - Name resolution only at connect: a DHCP address change is picked up on the next
        reconnect, not while the link is up.
      - Track net-snmp releases: the C build compiles a pinned tarball (version and SHA-256 in
        `impl/c/CMakeLists.txt`), so a security fix there means bumping the pin.
      - **Two devices on one host, different ports** are refused: `check_unique_hosts` compares
        the host literal alone, though an SNMP agent is addressed by host *and* port. The
        restriction exists because notifications are routed by source address, so it should
        apply to devices that have notification points, not to polled-only ones. Changing it
        means the spec (§3.2) and both implementations together, which is why it is not in the
        connector's first PR.
      - **v3 notifications through a forwarder** work in Rust but are refused by the C module
        ("v3 through a forwarder is not supported"), and no test covers it — the e2e forwarder
        relays v2c only. Either implement it in C or record it as a tagged capability gap.
      - **GETBULK for scalars** buys no round trip over a multi-varbind GET, and its
        GETNEXT semantics turn a missing instance into "the agent answered X instead of Y"
        rather than a plain noSuchObject. Reconsider `bulk = true` as the v2c/v3 default.
      - The receiver's `LocalEngine` state is not rolled back when a forwarded v3 notification
        is tried against a device it turns out not to belong to (`listener.rs`), unlike the
        per-device security state beside it.
      - **Push recovery ends when a re-subscribe fails** (`retain_pushed`, `impl/rust/crates/sdk/src/runtime.rs`):
        a device drops out of `pushed_devices` the moment it has no subscribed points, which is
        exactly when a failed re-subscribe means recovery should keep trying — so a mixed device
        whose subscription dies and fails to come back loses both retry paths at once (its polls
        keep succeeding, so `reconnects` stays clear too) and its push-only points go quiet until
        a restart. Not fixed here: keeping every configured device instead reconnects a healthy
        polled-only device once a minute forever, which the existing test
        `push_recovery_ends_when_a_device_falls_back_to_polling` pins deliberately. Telling the
        two apart needs `Connector::pushes_point` threaded into `retain_pushed` (both call sites),
        a change to shared runtime behaviour that wants its own PR.
      - **`subscribe = false` on a point a module can only push** is scheduled for polling, where
        `read_points` silently drops it: no sample, no bad sample, no warning. Not fixed in the
        runtime: having `push_points` override an explicit `subscribe = false` is worse than the
        drop, since it silently contradicts the operator. Both SNMP implementations already
        reject the combination in their own validation (`config.rs`, `connector_snmp.c`), which is
        where it belongs; the gap is only that the SDK does not *require* a module to do so.
      - **A transport error ends the whole device cycle in the C runtime**, for every connector:
        the poll loop in `impl/c/sdk/src/runtime.c` stops at the first read returning non-zero,
        so the remaining due points never publish the bad samples they already hold — and a
        consumer that simply stops hearing about a point cannot tell a stalled device from a
        value that has not changed. The SNMP module works around it for itself by holding the
        verdict back to the last point of a failed batch (`transport_pending` in
        `connector_snmp.c`); modbus, opcua and canbus still lose the rest of the tick. The Rust
        runtime publishes every sample of a failed batch before it judges the link, so fixing
        this in the C SDK would restore parity and let the SNMP workaround go. Only the SNMP
        e2e suite covers the path at all (`A Stopped Agent Gives Bad Samples And The Link
        Recovers`), which is why it went unnoticed.

* [ ] The `.apk` packages carry versions apk-tools rejects, for BOTH implementations and for
      real releases, not just snapshots: `apk version -c` reports `0.0.1-alpha.2` (this
      repository's existing tag format) and `0.0.0_pre.<sha>` (what nfpm derives from the
      snapshot version) as invalid, because apk's grammar allows `_pre1` but not `_pre.<hash>`
      and no bare `-alpha.2`. Valid forms are e.g. `0.0.1_alpha2` or `0.0.0~<sha>`. Fixing it
      means either an apk-specific version override in both packaging configs or a change to
      the tag convention. Nothing in CI installs an apk, which is why it has gone unnoticed —
      a `apk add --allow-untrusted` smoke on the built package would catch it.

* [ ] A simulator hook to delete an OPC UA subscription server-side while leaving the session
      up. It is the one push-failure path neither implementation can be tested against today
      (see the note in `impl/c/README.md`): open62541 reports the client as healthy throughout,
      so a regression there would be silent. `connectors/opcua/sim/` would need an endpoint or
      a method the suite can call.

* [ ] Fuzz the C parsers. The validation policy below requires a fuzz target for anything
      parsing external input; the Rust SDK has four (`just fuzz-all`), the C build has none,
      so its TOML loader (tomlc99) and DBC parser are only covered by the shared golden
      vectors and the e2e suites. libFuzzer via clang would reuse the same corpora.

* [ ] Cloud Fieldbus increments 3 + 4 (see `doc/rfc/0002-cloud-fieldbus-integration.md`;
      increments 1 + 2 shipped and verified live 2026-07-02): generalise the device-type
      translator per protocol, and the export path / UI-placeholder reconciliation (needs a
      tenant-side actor — a device cannot own or delete the UI-created managed object).
* [ ] Conformance suite implementation (`doc/conformance/conformance-suite.md` is spec'd,
      harness not built yet).
* [ ] File the upstream async-opcua issues (drafts ready in
      `doc/upstream/async-opcua-stranded-sample.md` and
      `doc/upstream/async-opcua-null-session-nonce.md`); drop `vendor/async-opcua-crypto`
      and the `[patch.crates-io]` entry once the nonce fix ships.
* [ ] c8y-fieldbus-import deferred items (script header TODOs): alarm/event/status mappings
      (gap G4), RTU serial-port resolution from `[connection.serial]`, signed and
      multi-register bit fields.
* [ ] Device parameters (RFC 0003, prototype implemented): persist the
      last-commanded value of write-only parameters across mapper restarts; derive
      `meta.parameter.set` from the Cloud Fieldbus device type name in `c8y-fieldbus-import`;
      optional Modbus FC16 fast path for `write-batch` on contiguous registers; optional
      `c8y_ParameterUpdate` audit event flow on top of the twin; gateway-level connector
      settings (poll_interval) as a tedge-parameter-plugin set script issuing `set-config`.
* [ ] Legacy write-payload compatibility (gap G2): accept explicit-address
      (`register`/`coil`/`address`/`ipAddress`) and name-based `metrics[]` payloads for
      `c8y_SetRegister`/`c8y_SetCoil`, not only `{point, value}`.

## Connector candidates

* ethercat — https://github.com/ethercrab-rs/ethercrab (MIT/Apache-2.0)
* EtherNet/IP — https://github.com/sergiogallegos/rust-ethernet-ip
* BACnet — spec sketch exists in `doc/connectors/_template-connector-spec.md`
* DNP3 — https://github.com/stepfunc/dnp3 — NOT possible (non-OSS license)

Score each implementation (functionality + maintainability incl. upstream library activity)
before promoting it past experimental.

## Validation policy

* Connectors must be validated with unit + integration tests and e2e simulator tests, and the
  tests must be proven by running them (see `doc/testing.md`).
* Shared SDK decode logic requires property-based tests; parsers of external input require a
  fuzz target.
