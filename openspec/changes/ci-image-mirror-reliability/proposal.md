## Why

The PR checks fail far more often from infrastructure than from code. Of the 21 failed jobs in the
last 12 failed CI / Publish-simulators runs (2026-10-12 → 2026-10-15), **17 failed on a container
pull from `public.ecr.aws`** with `429 Too Many Requests — toomanyrequests: Data limit exceeded`,
before a single test ran (C build & conformance, e2e, e2e-c, interop and `publish (profibus)` alike).
Every CI run on `main` since 0.0.16 has been red for this reason.

The mitigations already in place — moving off Docker Hub to ECR Public, `max-parallel: 2` on every
matrix, and a 5-attempt pull retry in `just _pull-stack-images` — were aimed at a *request-rate*
throttle (`Rate exceeded`). The current error is a *data-volume* quota for anonymous pulls, counted
per egress IP that GitHub-hosted runners share with every other Actions user. Retrying for 50 s
cannot outlast it, and throttling our own parallelism barely changes our share of the traffic. We
need to stop pulling third-party base images anonymously from shared-quota registries in CI.

The remaining failures are smaller but recurring: one flaky reload e2e test (modbus C,
`A Changed Service Name Restarts The Connector` timing out waiting for `status: down`), and
dependabot site PRs that can never pass because they bump `preact` to 11 while `@astrojs/preact`
still peers on `^10`.

## What Changes

- **Mirror the third-party images CI uses into this repository's GHCR namespace**
  (`ghcr.io/thin-edge/tedge-dot/mirror/<image>:<tag>`) with a new scheduled + manually triggerable
  workflow. The list of mirrored images lives in one file that CI checks against the repo, so a new
  `FROM`/`image:` that is not mirrored fails a cheap lint job instead of an e2e run.
- **CI resolves base images from the mirror.** Every Dockerfile already takes `BASE_REGISTRY`; the
  compose files' registry `image:` lines (mosquitto, alpine) become `${BASE_REGISTRY:-…}/…` too, and
  compose passes `BASE_REGISTRY` as a build arg. CI sets `BASE_REGISTRY` once at workflow level.
  Local runs keep today's default (`public.ecr.aws/docker/library`), so nothing changes for
  developers.
- **Authenticated GHCR pulls** in CI via `docker/login-action` with `GITHUB_TOKEN` (read-only, also
  available to fork and dependabot PRs).
- **Fallback, not a hard dependency:** `_pull-stack-images` (and a new base-image pre-pull for
  builds) tries the mirror first, then upstream ECR, then Docker Hub, and tags whatever it got under
  the reference compose/BuildKit expects; 429s back off with a longer, jittered schedule.
- **`publish-simulators.yaml`** builds its images from the mirror as well (build arg), so PR builds
  of the simulators stop failing on the same quota.
- **Relax the `max-parallel: 2` caps** on the C/conformance/e2e/e2e-c matrices once the mirror is
  proven, cutting wall-clock time (the caps were only ever a rate-limit workaround).
- **Harden the flaky modbus reload test** (service rename → `down` health status) so it waits on
  the actual condition rather than a fixed window.
- **Dependabot:** ignore `preact` major updates in `/site` until `@astrojs/preact` supports 11, so
  the weekly group PR is mergeable.

## Capabilities

### New Capabilities
- `ci-container-images`: how CI obtains third-party container images — a repository-owned GHCR
  mirror, the single list of mirrored images and its consistency check, the `BASE_REGISTRY`
  override contract shared by Dockerfiles and compose files, and the pull/fallback/retry behaviour.

### Modified Capabilities
<!-- None: no existing spec covers CI; the connector behaviour specs are unaffected. -->

## Impact

- **Workflows:** new `.github/workflows/mirror-images.yaml`; `ci.yaml` (env, GHCR login, matrix caps,
  new lint job), `publish-simulators.yaml` (build args), `cloud-e2e.yaml` (same env/login, it builds
  `cloud/modbus/Dockerfile.tedge`).
- **Build files:** compose files under `connectors/*/` (and `opcua/interop`, `modbus` multi-device,
  `opcua` namespaces/secure variants) get a parametrised broker/alpine image and a `BASE_REGISTRY`
  build arg; Dockerfiles are unchanged apart from comments.
- **justfile:** `_pull-stack-images` resolves images via `docker compose config --images` (instead
  of grepping `image:`), gains mirror→upstream fallback and longer 429 backoff; new
  `check-mirrored-images` recipe.
- **Repository settings:** GHCR packages under `thin-edge/tedge-dot/mirror/*` must be public
  (fork PRs pull them anonymously when login is unavailable). One-time manual step.
- **Tests:** modbus reload e2e suite; no connector code changes.
- `.github/dependabot.yml`.
- Out of scope: release workflow failures (goreleaser "dirty" at 0.0.15, already resolved by later
  releases) and the Rust e2e `rust:1-bookworm` build volume (a shared prebuilt connector image is a
  possible follow-up once the mirror removes the quota pressure).
