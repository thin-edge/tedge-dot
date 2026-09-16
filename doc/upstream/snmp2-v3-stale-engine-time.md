# Upstream issue draft: snmp2 v3 requests carry a stale engine time

Target: https://github.com/roboplc/snmp2 (crate `snmp2` 0.5.2)

## Summary

`AsyncSession::prepare` (and `SyncSession`'s) calls `Security::correct_authoritative_engine_time`,
which computes `AuthoritativeState::engine_time_current` = learned time + local seconds elapsed.
Nothing reads `engine_time_current`: `v3::build` writes `security.engine_time()` — the value
learned at discovery or from the last authenticated response — into msgAuthoritativeEngineTime,
and `encrypt_aes` builds the IV from the same stale value.

As long as requests follow each other within 150 s, every response refreshes the learned time
and nothing shows. A request sent more than 150 s after the previous exchange carries a time
more than 150 s behind the agent's clock: the agent answers with an authenticated
`usmStatsNotInTimeWindows` Report, `parse_v3` updates the learned time and then fails the
request with `EngineTimeMismatch` (its own window check compares against the stale previous
value). The next request, again more than 150 s later, fails the same way — so a device polled
every 5 minutes over v3 never returns a value.

## Reproduction

1. `AsyncSession::new_v3(..)` with authNoPriv against net-snmp `snmpd`; `init()`.
2. `get(sysUpTime.0)` → Ok.
3. Wait 160 s; `get(sysUpTime.0)` → `Err(AuthFailure(EngineTimeMismatch))`, and snmpd's
   `usmStatsNotInTimeWindows` counter increments. Repeat step 3: the same.

## Proposed fix

Send (and derive the AES IV from) the estimated current time: the learned time plus the local
seconds elapsed since it was learned, capped at 2^31−1 — i.e. use `engine_time_current`, computed
at build time so callers that skip `prepare` (e.g. `Pdu::to_bytes_with_security`) are covered.
Handling a `notInTimeWindows` Report by resynchronising and resending once (as net-snmp does)
would make the client robust against clock drift as well.

## Local workaround

tedge-dot vendors snmp2 with the estimated time (`impl/rust/vendor/snmp2`, patch 6; test
`a_request_carries_the_estimated_engine_time`), and retries a request once after an
`EngineTimeMismatch` / `AuthUpdated`. Found on 2026-09-15 while implementing SNMPv3 polling.
