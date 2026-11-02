## Context

A connector reads every device and every block one after another:

```
one loop, one module object
 pass:  plc1 [blk][blk]  plc2 [blk][blk]  ...  plc50 [blk]       each [blk] = one round trip
 ceiling = points per block / (round trip + server time), whatever the CPU
```

- Rust: `Connector::read_points(&mut self, ...)`, so the runtime calls one read at a time for all
  devices (`runtime.rs`, the poll path). The OPC UA module's `read_in_blocks` awaits each 500-node
  Read before sending the next, although `async_opcua::Session::read` takes `&self` on an
  `Arc<Session>` and can have several Reads in flight.
- C: one serial loop with blocking module calls; MQTT runs in the same thread. A frozen device
  delays every other device (memory: C loop device starvation). The vendored open62541 offers
  `UA_Client_sendAsyncReadRequest`, driven by `UA_Client_run_iterate`, which the module already
  calls for subscriptions. The SNMP module already uses `snmp_sess_async_send`.

### Measurements (2026-11-02, M1 Max, scratch tooling, `--no-record`)

**Across devices.** The bench module with `latency = "20ms"` per `batch = 500` points, 64 devices
of 1,000 points, `poll_interval = "1s"`, measurement output, target 64,000 readings/s. Separate
connector processes stand in for concurrent reads:

| Instances | Rust | C | Connector CPU |
| --- | --- | --- | --- |
| 1 | 22,917/s | 19,667/s | 4–7 % |
| 2 | 46,167 | 39,042 | 7–14 % |
| 4 | 58,417 | 59,667 | 7–19 % |
| 8 | 57,667 | 62,375 | 7–19 % |

Throughput scales almost linearly until the pass fits the poll interval (64 devices × 2 blocks ×
20 ms needs about three reads in parallel). The connector is idle, waiting. Rust's remaining 9 %
shortfall at 4 and 8 instances is the late-poll issue, not this one.

**Within a device.** A scratch `async-opcua` client read 500-node blocks with K Reads in flight,
through a proxy that delays each direction by half the round trip.

The bench's server (`ot-conformance serve opcua`, async-opcua, multi-threaded), 20,000 nodes:

| K | 0 ms round trip | 20 ms | 100 ms |
| --- | --- | --- | --- |
| 1 | 130,351/s | 19,484/s | 4,735/s |
| 4 | 535,867 | 77,278 | 18,519 |
| 8 | 911,868 | 161,698 | 37,536 |
| 64 | 2,926,775 | 664,399 | 185,791 |

The e2e simulator (Python `asyncua` in its container, single-threaded, about 5 ms per 500-node
Read), 5,000 nodes, standing in for a small PLC that serves one request at a time:

| K | 0 ms round trip | 20 ms |
| --- | --- | --- |
| 1 | 57,627/s | 14,704/s |
| 2 | 84,829 | 32,025 |
| 4 | 94,048 | 52,239 |
| 8 | 95,046 | 53,746 |
| 16 | 77,699 | 65,152 |

- Against a serial server, four Reads in flight reach its ceiling; more only queue, and 16 was
  slower than 4 with no network delay. Against a parallel server, more keeps paying.
- Neither server refused a request, even at K = 64. The backoff (D6) is therefore not validated by
  these servers; the bench module's server limit (D8) is how it gets tested.
- Connector CPU stayed at 4–19 % in every run: the work is waiting, so the gateway's CPU count is
  not what limits it (D4).

## Goals / Non-Goals

**Goals:**
- One connector reaches 50,000 readings/s or more with 20 ms per block, against about 23,000 today.
- A device is never read concurrently with itself, and its results keep their order and read time.
- No endpoint receives more requests at once than its budget, and an endpoint that says it is busy
  gets fewer.
- One frozen device no longer delays the others.
- The same behavior in both builds.

**Non-Goals:**
- Pipelining for Modbus (libmodbus is synchronous; most devices serve one transaction at a time),
  CANopen SDO (one at a time per node), PROFIBUS and CAN (cyclic or pushed).
- Concurrent writes or method calls. They keep their current order and wait for a device's read.
- The late-poll shortfall (a separate change), although shorter passes may reduce it.
- Concurrency inside the runtime's state (report policy, batcher, link status, MQTT).

## Decisions

### D1. Concurrency only around the module's read call
The runtime's state stays on one loop. A read is started on a worker (a task in Rust, a thread in
C), and its result comes back to the loop, which applies the report policy, the batcher, the link
status and publishing exactly as today.
- Nothing else becomes concurrent, so the existing tests and vectors keep their meaning.
- *Rejected: running the whole per-device path concurrently.* Every piece of runtime state would
  need locking.

### D2. Rust: a per-device reader handed out by the module
A new optional `Connector` method returns, for a device, a `DeviceReader` (`Send`, owning what the
read needs, such as the OPC UA module's `Arc<Session>`) whose `read_points` can be moved into a
task. The runtime asks for a reader after each connect or reconnect and drops it when the device's
link is lost. A module without the method is read as today, one call at a time through the module
object.
- Modules opt in one at a time: OPC UA, Modbus, SNMP and bench first.
- *Rejected: changing every `Connector` method to `&self`.* That refactors all seven modules at
  once for the same result.
- *Rejected: one module instance per device or per shard.* Modules share per-connector resources,
  such as SNMP's trap listener and the CAN socket of an interface, which would conflict.

### D3. C: worker threads, with a module flag
A module sets `concurrent_reads` when its `read_points` (and `read_point`) are safe to call at the
same time for different devices. The runtime then runs reads on a pool of worker threads, at most
`max_inflight_total`, and the main loop collects results from a queue. A pipe wakes the main loop,
so `mosquitto_loop` doesn't wait out its timeout. Without the flag, reads stay on the main loop.
- OPC UA (one `UA_Client` per device), Modbus (one `modbus_t` per device) and SNMP (net-snmp's
  single-session API, `snmp_sess_*`) are the candidates. SNMP's file-level
  `tsnmp_parse_session` is used for parsing only and must be checked or made per-call.
- A device's writes, commands and reconnects take the device's lock, so they wait for its read.
- *Rejected: non-blocking I/O in every module.* A rewrite of each module.

### D4. A per-endpoint budget, not a CPU-based default
- Every device has an **endpoint**, named by its module: an OPC UA endpoint's host and port,
  Modbus TCP's host and port, the serial port for Modbus RTU, an SNMP agent's host and port, the
  CAN interface for CAN and CANopen, the PROFIBUS bus. A module without the hook uses the device
  name, so each device is its own endpoint.
- The module also gives a **cap** for an endpoint: 1 for a serial or CAN bus, unlimited otherwise.
- `max_inflight` (default 4), on `[connector]` or a device, is the number of requests in flight
  to one endpoint. Devices that share an endpoint use the smallest of their values; the module's
  cap always wins.
- `max_inflight_total` (default 32), on `[connector]`, caps requests in flight across all
  endpoints. It bounds the C build's worker threads.
- Four is where the serial server above stops gaining, and a parallel server still gains three to
  four times over one. Connector CPU stayed under 20 % in every measurement, so the gateway's CPU
  count is not used: the budget protects the remote device, which more gateway cores do not make
  faster.

### D5. Sharing the budget between devices and blocks
A read of a device takes one permit of its endpoint, waiting if none is free. It then takes up to
`allowance - 1` more without waiting, where the allowance is what the module says the call can use
(its blocks, at most the budget). The module sends at most that many requests at once, and
returns all permits when the call ends.
- Every device on an endpoint makes progress: none waits for another's pipelined read to finish
  before getting its first permit.
- A module that cannot pipeline always asks for 1.
- The total cap is a second semaphore, taken the same way.

### D6. Backoff on a busy endpoint
An endpoint's effective budget halves (to at least 1) when a request is answered with OPC UA
`BadTooManyOperations`, `BadServerTooBusy` or `BadTooManySessions`, or times out while another
request to the endpoint is in flight. After 20 successful requests at the reduced budget, it grows
by one, up to `max_inflight`. Each change is logged at `info`.
- A request refused as busy is sent again within the same call once the budget allows, until the
  call's `operation_timeout`. Only then do its points fail, so a busy server slows a pass down but
  does not produce `bad` readings.
- With a server that serves exactly N at a time, the budget oscillates between about N/2 and N
  (halve on a refusal, one step back every 20 successes), like a congestion window.
- A timeout with nothing else in flight is an ordinary link failure, handled as today.
- The current effective budget is visible in the capability descriptor.

### D7. Results keep their order and read time
A pipelined call reassembles block results by index. A block that fails fails only its own points.
A lost link stops sending, ends the call after the blocks in flight, and reports
`ConnectorError::LinkLost`, as `read_in_blocks` does today. The runtime stamps the call's read time
when the whole call completes (contract §5.4), so a device's pass is still one measurement under
`group_by = "time"`.

### D8. The bench models endpoints, pipelining and a busy server
The bench module's `protocol_address` gains:
- `endpoint`: devices with the same value share an endpoint.
- Pipelining: blocks within the call's allowance overlap their latency.
- `server_inflight`: the most blocks the simulated endpoint serves at once. A block beyond it is
  answered "busy", which tests D6.

The `polled-latency` scenario: 64 devices on separate endpoints, plus one 20,000-point device,
20 ms per 500-point block, measurement output.

### D9. Contract 0.4.0
New keys (`max_inflight`, `max_inflight_total`) and a capability-descriptor field, so the minor
version goes up.

## Performance

Targets, on `polled-latency` (64 devices × 1,000 points, 20 ms per 500-point block), one
connector instance, measurement output, same host as the measurements above:

| Metric | Today (one instance) | Target (one instance, defaults) |
| --- | --- | --- |
| Readings/s at a 64,000/s target | 22,917 (Rust), 19,667 (C) | ≥ 58,000 in both builds: the four-instance figure |
| One 20,000-point OPC UA device at 20 ms round trip | 19,484 | ≥ 70,000 with K = 4 against `ot-conformance serve opcua` |
| Connector CPU | 4–7 % | ≤ 25 % |
| A busy endpoint (`server_inflight = 2`) | — | no `bad` readings from refusals; the effective budget stays between 1 and 3 |

The before lines are recorded in `bench/results/` by the first task, from the same host.

## Risks / Trade-offs

- **[A small PLC is overloaded by default]** → Four in flight reached the serial server's
  ceiling without errors. The backoff halves on "busy" or a timeout with other requests in flight,
  and `max_inflight = 1` restores today's behavior per endpoint.
- **[Timeouts count queueing time]** → With K requests queued at a serial server, the last waits
  for the K - 1 before it. `operation_timeout` (5 s) leaves ample room at K = 4 even at 100 ms per
  request. A timeout with others in flight backs off rather than marking the link lost.
- **[Module thread safety in C]** → Opt-in per module (D3). A module is flagged only after review,
  and the CI runs its e2e suite with concurrency on and under AddressSanitizer in the unit tests.
- **[Two logical devices on one PLC]** → They share the endpoint's budget (D4), so the PLC sees
  the same at most 4 as with one device.
- **[Session limits]** → Concurrency is per device's existing session; no new sessions are opened.
- **[The C build's threads]** → At most `max_inflight_total` workers, created on demand.

## Migration Plan

With the defaults, devices on different endpoints are read in parallel and OPC UA devices send up
to four Reads at once. A site that needs today's behavior sets `max_inflight = 1` on
`[connector]`. Downgrading restores the serial loop. Nothing is stored.

## Open Questions

- SNMP pipelining in the Rust build depends on `snmp2`'s async API (vendored); to be checked in
  the first SNMP task. If it does not allow several requests in flight, Rust SNMP gets concurrency
  across agents only.
