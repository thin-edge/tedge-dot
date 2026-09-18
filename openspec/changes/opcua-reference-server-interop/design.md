# Design

## Context

See `proposal.md` — Why. The constraints that shape this design come from the upstream project
and from our existing test conventions:

- **Upstream compose is unusable as shipped.** `php-opcua/uanetstandard-test-suite`'s
  `docker-compose.yml` builds every service from source against the .NET 10 *preview* SDK and
  publishes fixed host ports 4840–4852 plus UDP 14850. Our stacks use DeviceLibrary compose
  projects with random names and ephemeral ports, and `connectors/_shared/stack.resource`
  rejects fixed host ports so suites can run in parallel.
- **The published image is `linux/amd64` only** (`ghcr.io/php-opcua/uanetstandard-test-suite`),
  and it does not merely fail to run on arm64 — under qemu it aborts during startup with a
  .NET `AccessViolationException` inside `FrozenDictionary`, a JIT/emulation incompatibility.
  Verified on this project's Colima VM: the server prints its whole configuration, loads its
  users, then dies. There is no emulation path.
- **The image is fully env-driven**: `OPCUA_PORT`, `OPCUA_RESOURCE_PATH`,
  `OPCUA_SECURITY_POLICIES`, `OPCUA_SECURITY_MODES`, `OPCUA_AUTO_ACCEPT_CERTS`,
  `OPCUA_ALLOW_ANONYMOUS`, `OPCUA_AUTH_USERS`, `OPCUA_AUTH_CERTIFICATE`,
  `OPCUA_SECURITY_TOKEN_LIFETIME`, `OPCUA_IS_DISCOVERY` and friends. One image, many profiles.
  A single instance offering all six policies serves **11 endpoints**, measured.
- **The server advertises `opc.tcp://0.0.0.0:<port><path>` as its endpoint URL.**
  `BuildBaseAddresses` in `src/TestServer/Program.cs` hardcodes `0.0.0.0` and ignores the
  `OPCUA_HOSTNAME` it parses. A client that dials the advertised URL fails; `asyncua` gets
  `BadServerUriInvalid` on CreateSession. This is not a problem to work around but the single
  best real-world case for the "endpoint selection keeps the configured address" requirement.
- **The server's own certificate is self-signed and regenerated on every process start.** Its
  `SetupPkiDirectories()` deletes `/tmp/pki/own` at startup, and the CA-signed
  `certs/server/cert.pem` on the host is *not* what it presents. So nothing about it can be
  pre-pinned, and its thumbprint changes across a container restart.
- **The server reads its trusted-client store only at startup**, copying `certs/trusted/*` into
  `/tmp/pki/trusted/certs`. Its rejected store is hardcoded to `/tmp/pki/rejected` inside the
  container, so inspecting it needs `docker exec`.
- **Our node addressing is namespace-index-only.** `connector-opcua`'s `NodeAddress` accepts
  `node_id = "ns=2;s=Temperature"` or `namespace` + `identifier`
  (`impl/rust/crates/connector-opcua/src/config.rs:146`); there is no namespace-URI form. The
  reference server's docs state its namespace indices are typical, not guaranteed.
- **Both implementations must be covered** through the existing `IMPL` switch, and
  `Connector Implementation Should Be` already guards against a silent fallback to Rust.

## Goals / Non-Goals

**Goals:**
- Prove both connectors against a stack we did not write, without giving up anything the
  current suites cover.
- Keep the interop stack off the default local loop, and keep CI deterministic against a young
  upstream project.
- Fix the reason-category defect this work exposes, in both implementations, with the same
  text.

**Non-Goals:**
- Replacing `sim/server.py` or the embedded conformance server.
- ECC policy support, PubSub/UADP, HTTPS transport, reverse connect, SKS, file transfer.
- Namespace-URI addressing in the connector (see Risks — it is a real gap, deferred).
- Testing the reference server's own correctness, or working around its documented defects.

## Decisions

### Our own compose file, not upstream's compose or their GitHub Action
`connectors/opcua/interop/docker-compose.yaml` declares the server services ourselves, from the
pinned published image, with `OPCUA_*` env vars and no fixed host ports.

*Alternatives:* (a) upstream `docker-compose.yml` with an override file — the overrides would
have to unset thirteen port mappings and switch every service from `build:` to `image:`, which
is more surface than writing five services; (b) their composite Action
`php-opcua/uanetstandard-test-suite@v1` — it checks the repo out and runs
`docker compose up --build`, so it builds from source, keeps the fixed ports and does not
produce a DeviceLibrary-managed project. Both were rejected for the same reason: we would
inherit a build and a port map we then have to fight.

### We build the reference server ourselves, multi-arch, from a pinned upstream tag
The upstream image cannot run on arm64 at all (see Context), which would have made the suite
unrunnable and unverifiable on the maintainers' own machines and forced every fix through CI.
Both of upstream's base images (`mcr.microsoft.com/dotnet/{sdk,aspnet}:10.0-preview-alpine`)
publish `linux/arm64`, so we build their unmodified `Dockerfile` from a pinned git tag for
`linux/amd64` and `linux/arm64` and publish it as
`ghcr.io/thin-edge/tedge-dot/uanetstandard-refserver`, exactly as the project already publishes
`opcua-sim`. Verified: the arm64 image builds and the server starts and serves 11 endpoints
natively.

The upstream source is not vendored — the build context is the pinned git ref, so we ship none
of their code and carry no patches. Pinning the ref, rather than tracking a tag, means an
upstream release cannot silently change what CI proves; bumping it is a reviewed commit.

*Alternatives:* (a) pin their published amd64 digest — rejected, it makes the suite CI-only and
undebuggable locally on Apple Silicon, which is what the maintainers develop on; (b) run it
under emulation — rejected, it does not work; (c) fork and patch the server — rejected, we want
their behaviour, including its defects, since that is what makes it a useful interop target.

### Five server profiles, not thirteen
The upstream compose runs thirteen instances because it is a catalogue. We only need the
distinct *server-level* configurations, since one instance can offer many policies at once:

| Service | Configuration | Covers |
|---|---|---|
| `ref-all` | every supported policy, all modes, anonymous + users + certificate auth, auto-accept on, resource path set | the policy/mode matrix, `sign` vs `sign_and_encrypt`, deprecated policies, all three identity types and their rejections, typed reads, subscriptions |
| `ref-strict` | `Basic256Sha256`/`SignAndEncrypt`, `OPCUA_AUTO_ACCEPT_CERTS=false` | the server rejecting our application certificate |
| `ref-ecc` | ECC policies only | the clean `no matching endpoint:` refusal |
| `ref-token` | `OPCUA_SECURITY_TOKEN_LIFETIME` at its minimum | secure channel token renewal |
| `ref-discovery` | `OPCUA_IS_DISCOVERY=true`, empty `OPCUA_RESOURCE_PATH` | an endpoint with no resource path |

A "server that offers only `Sign`" is not a separate instance: requesting `sign` against
`ref-all` still negotiates a Sign channel on the wire, and the *endpoint-selection* behaviour
when only Sign is offered is already covered by our own simulator. Five .NET containers is
also a meaningful memory saving over thirteen in CI.

### Trust bootstrapping runs in two independent directions
- **Us → them** is the operator flow and needs no pre-staging, which suits a certificate that
  changes on every start: connect, assert the device is quarantined with `certificate
  untrusted:`, read the thumbprint from `tedge-dot pki list rejected --json`, run `tedge-dot
  pki trust <thumbprint>` in the connector container, assert the device reaches `connected`.
  That is exactly the documented flow and exercises the CLI as a side effect.
- **Them → us** only matters for `ref-strict`. The connector's application certificate is
  created up front with `tedge-dot pki create` in suite setup and exported into a shared volume
  that `ref-strict` mounts at `/app/certs`, so its startup copy picks it up. Because the server
  reads that store only at startup, the "now the server trusts us" half of the test restarts
  `ref-strict` rather than expecting a live reload.

  That restart regenerates the server's own certificate, which would break a thumbprint we had
  pinned — so the `ref-strict` device sets `trust_any_server_certificate = true`. That test is
  about *their* judgement of *us*; our judgement of them is covered thoroughly elsewhere, and
  pinning here would only make the test flaky for a reason unrelated to what it asserts.

### The suite renders the connector config from a template
Because the connector cannot address a node by namespace URI, suite setup queries the server's
namespace array once, resolves the index for `urn:opcua:testserver:nodes`, and renders
`connector.toml` from a template before the connector container starts. The query reuses the
`asyncua` client already present in the `opcua-sim` image rather than adding a dependency.

*Alternative:* hardcode `ns=1`. Rejected — it is what the upstream docs explicitly warn
against, and it would make the suite fail for a reason that has nothing to do with our
connector.

### `application certificate rejected:` is decided by ordering, not by status code
`BadSecurityChecksFailed` and `BadCertificateUntrusted` are returned both when we distrust the
server and when the server distrusts us, so the status code alone cannot tell them apart
(`impl/rust/crates/connector-opcua/src/security.rs:25`). The connector already validates the
server certificate as a pre-check before the channel opens. The rule is therefore positional:
if that pre-check passed — or was skipped by `trust_any_server_certificate` — and the handshake
or activation then fails with one of those codes, it is the server rejecting us. Both
implementations already have the pre-check, so both get the same three-line change.

### Layout keeps the interop suite out of the default run
`just test-e2e opcua` runs everything under `connectors/opcua/tests/`, so the interop suite
lives in `connectors/opcua/interop/` with its own `tests/` directory, compose file, config
template and entrypoint. New recipes `just test-interop opcua` and `just test-interop-c opcua`
mirror `_e2e`, including the `IMPL` export and the missing-capability skips.

### No architecture gate
Building both architectures removes the gate the design previously needed: the suite runs the
same way on an arm64 laptop and an amd64 runner, so there is no skip path to get wrong and no
class of failure that only CI can see.

## Risks / Trade-offs

- **Upstream is young (1 star, one maintainer, AI-assisted) with known doc/code mismatches** —
  `OPCUA_REJECTED_CERTS_DIR` is parsed but unused, `OPCUA_HOSTNAME` likewise, the ECC services
  are documented as strict but shipped with auto-accept on, and `trust-flow.md` claims a server
  certificate stability the code contradicts. → Pin by digest, keep our own simulator as the
  default e2e server, and assert behaviour we observed in their code rather than what their
  docs claim. The interop job failing must never be able to block the `e2e` job.
- **Their "expired" certificate is not expired for its first ~24 h** (generated with `-days 1`,
  not backdated). → We do not use it. Expiry stays covered by `genpki.py`'s vector, which is
  backdated properly.
- **Their servers run deliberately lax** — `MinimumCertificateKeySize = 1024`,
  `RejectSHA1SignedCertificates = false`. → Only relevant if we asserted the server enforces
  something; we do not. Our own key-length rules are client-side and tested against our
  simulator.
- **We now own a container build.** If upstream's `Dockerfile` or its .NET preview base changes
  incompatibly, our build breaks rather than silently drifting. → The ref is pinned, so this can
  only happen when someone bumps it, and the publish workflow runs on that commit.
- **Five .NET containers add CI time and memory.** → Own job, so it does not lengthen the
  critical path of `e2e`; `ref-ecc`, `ref-token` and `ref-discovery` are small single-purpose
  instances.
- **Namespace-URI addressing is a genuine connector gap** that this work exposes: a real server
  may reorder its namespace array between firmware versions and silently break a configuration
  that hardcodes `ns=1`. The suite works around it by rendering config, which proves the
  connector works but not that an operator could cope. → Out of scope here; record it in
  `TODO.md` as a follow-up for an `nsu=`/`namespace_uri` address form.
- **A restart of `ref-strict` mid-test is a synchronisation point** that could flake if the
  suite races the server's startup. → The service keeps a healthcheck and the suite waits on it
  before asserting, the same way the secured stack waits on `simulator-secure`.

## Migration Plan

Additive: no runtime behaviour changes for existing configurations except the new reason
category, which only narrows a case previously reported under `certificate untrusted:`. Any
operator tooling matching on that prefix for a server-side rejection would need to match the new
one — worth a line in the changelog, but the old text was actively misleading, so keeping it is
not an option. Rollback is removing the interop job and directory; the two connector fixes stand
on their own and are covered by the modified `opcua-secure-connections` spec.

## Open Questions

- Which upstream digest to pin initially — resolved when the first task pulls it; it does not
  change the approach.
- Whether the token-lifetime minimum the server actually grants is short enough to observe a
  renewal within a reasonable test timeout. If it clamps the requested value upward, the
  renewal test may need a longer run or a `--` flag; either way the requirement and the task
  breakdown stay as they are.
