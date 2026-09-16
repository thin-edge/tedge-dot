# Upstream issue draft: snmp2 AsyncSession has no timeout, and a late response fails the next request

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

`AsyncSession` (`src/asyncsession.rs`) sends a request and awaits `socket.recv` with no bound:
over UDP a lost request or response makes `get`/`get_many`/`getbulk`/`set` wait forever, and
there is no retry. (`SyncSession` has a timeout; the async one does not.)

The natural workaround, `tokio::time::timeout(d, session.get(..))`, exposes a second problem.
When the response to the abandoned request arrives late, it stays queued on the connected
socket; the next request reads it, `Pdu::validate` fails it with `RequestIdMismatch`, and the
real response is left queued for the request after that. One late datagram can fail every
following request as long as each finds the previous answer in the queue.

## Reproduction

1. An agent that answers every request twice: first a Response carrying `req_id - 1`, then the
   real one (equivalent to a late answer to the previous request).
2. `session.get(&oid).await` → `Err(RequestIdMismatch)` on every call after the first.

With a real agent: poll with `timeout(100ms, session.get(..))` against an agent that answers in
150 ms.

## Proposed fix

- In each request method, keep receiving until a datagram parses as a Response to *this*
  request-id; skip (and optionally count) the others instead of failing. Parse errors still end
  the request.
- Offer a timeout and retry count on the session (`AsyncSession::set_timeout(Duration)`,
  `set_retries(u32)`), resending the same request-id so a late answer to an earlier attempt is
  accepted.

Note that the request-id is only incremented after a successful parse, so a request cancelled by
an outer timeout is re-sent with the same id by the next call — convenient for retries, but
worth documenting.

## Local workaround

tedge-dot vendors snmp2 with the skipping loop (`impl/rust/vendor/snmp2`, patch 4; test
`a_late_response_to_an_earlier_request_is_skipped`) and applies its own `request_timeout` and
`retries` around every call. Found on 2026-09-15 while implementing SNMP polling.
