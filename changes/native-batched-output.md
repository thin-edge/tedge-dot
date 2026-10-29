### Measurement output: batched measurements from the connector

A connector can now publish thin-edge measurements itself, hundreds of readings per message,
instead of one sample per reading for the `ot-measurement` flow to convert. Set
`output = "measurements"` in the `[connector]` table. No flow runs for a measurement, which takes
most of the work off the mapper at high reading rates. See
[doc/measurement-output.md](../doc/measurement-output.md).

- Opt-in: without `output`, a connector publishes samples exactly as before.
- `batch = { group_by = ... }` chooses how readings with different timestamps are grouped:
  `"time"` (default, nothing moved or lost), `"frame"` (a window) or `"sample"` (a fixed rate).
- Samples are still published for points with alarms, events or parameters, and for points with
  `sample = true`.
- Pushed OPC UA values in the C build now carry the server's source timestamp, as in the Rust
  build. In both builds, every sample of one read call has the same `ts`.
- **Action needed if you use the measurement-series alarm or event** (a `series` in the
  `ot-alarm` or `ot-event` `params.toml`): that mode moved to the new `ot-alarm-series` and
  `ot-event-series` flows. The package installs the new flow with your params file on upgrade;
  check that `/etc/tedge/mappers/c8y/flows/ot-alarm-series/` (or `ot-event-series/`) exists.
- The OT Connector Contract is now version 0.2.0 (§5.4, §5.5, `measured`, `outputs`).
