## Context

CI (`ci.yaml`) runs six-protocol matrices for C build & conformance, Rust conformance, e2e (Rust)
and e2e-c, plus the two-job OPC UA interop suite. Every e2e/C job starts Docker stacks
(`connectors/<proto>/docker-compose.yaml`, started per suite by DeviceLibrary) and builds the
connector, simulator and flows images locally. Third-party images come from two places:

- **`FROM ${BASE_REGISTRY}/…`** in every Dockerfile (`rust:1-bookworm`, `debian:bookworm`,
  `debian:bookworm-slim`, `python:3.11`, `python:3.11-slim`, `python:3.11-slim-bookworm`,
  `python:3.12-slim`), defaulting to `public.ecr.aws/docker/library`.
- **literal `image:` lines** in compose files: `eclipse-mosquitto:2` (every stack) and `alpine:3`
  (opcua interop), hard-coded to `public.ecr.aws/docker/library`. The interop server image is
  already ours on GHCR.

Earlier mitigations (ECR instead of Docker Hub, `max-parallel: 2`, 5× retried pre-pull with 5–20 s
backoff in `just _pull-stack-images`) targeted `Rate exceeded`. The failures since 2026-10-12 are
`429 toomanyrequests: Data limit exceeded` — ECR Public's anonymous *bandwidth* quota per source IP.
GitHub-hosted runners share egress IPs with all Actions users, so our own volume is not what
exhausts it, and the quota does not recover in seconds. The heaviest image, `rust:1-bookworm`
(~0.5 GB compressed), is pulled by every Rust e2e job.

## Goals / Non-Goals

**Goals:**
- No PR check depends on an anonymous pull from a shared-quota registry (ECR Public, Docker Hub).
- Developers' local runs keep working unchanged, with no GHCR login.
- If the mirror is ever unavailable, CI degrades to today's behaviour rather than failing outright.
- Adding a new base image without mirroring it is caught by a fast lint job, not by a red e2e run.
- Fix the two non-registry recurring failures (flaky modbus reload test, unmergeable dependabot
  site PRs).

**Non-Goals:**
- Pinning base images by digest (the mirror refreshes floating tags weekly; digest pinning is a
  separate reproducibility decision).
- Sharing prebuilt connector images across jobs / caching `docker save` tarballs. Worth doing for
  speed later; not needed for reliability once pulls come from GHCR.
- Release workflow failures (goreleaser "dirty" at 0.0.15 is already resolved).
- The cloud e2e base image `ghcr.io/thin-edge/tedge-demo-main-systemd` (already GHCR).

## Decisions

### 1. Mirror into this repository's GHCR namespace

`ghcr.io/thin-edge/tedge-dot/mirror/<name>:<tag>`, e.g. `…/mirror/eclipse-mosquitto:2`, so that
`BASE_REGISTRY=ghcr.io/thin-edge/tedge-dot/mirror` is a drop-in replacement for
`public.ecr.aws/docker/library` — the same `<name>:<tag>` suffix everywhere.

*Why GHCR:* pulls from Actions runners are authenticated with the always-available `GITHUB_TOKEN`
(including fork and dependabot PRs, read-only), stay on GitHub's network, and have no published
anonymous data quota. Public packages are free to store.

*Alternatives:* authenticated Docker Hub / ECR pulls need secrets that fork and dependabot PRs do
not get — the checks most exposed to the problem. A pull-through cache (self-hosted registry) is
infrastructure we would have to run. `actions/cache` of `docker save` tarballs still needs one
successful upstream pull per cache key and adds 1–2 GB of cache churn per run.

### 2. One list of mirrored images, enforced by a lint job

`.github/mirror-images.txt` holds one `<name>:<tag>` per line. `mirror-images.yaml` (weekly
schedule + `workflow_dispatch` + push to `main` touching the list) copies each entry with
`docker buildx imagetools create --tag <mirror> <upstream>`, which preserves the multi-arch index
(the simulators publish arm64 too, and developers on arm64 may set `BASE_REGISTRY` to the mirror).
Upstream source is ECR Public with a Docker Hub fallback; each image retries with long backoff, and
a failed copy leaves the previous mirror tag in place (the job fails visibly but CI is unaffected).

`just check-mirrored-images` greps `FROM ${BASE_REGISTRY}/X` from all Dockerfiles and
`${BASE_REGISTRY:-…}/X` from all compose files and fails if any `X` is missing from the list. It
runs as a job next to `capability-tags` / `packaging-parity`.

*Alternative:* deriving the list on the fly in the mirror workflow. Rejected: the explicit list
makes the set reviewable and lets the lint point at the exact missing line.

### 3. `BASE_REGISTRY` is the single switch, for builds *and* compose images

Compose registry images become `image: ${BASE_REGISTRY:-public.ecr.aws/docker/library}/eclipse-mosquitto:2`,
and every `build:` gets `args: { BASE_REGISTRY: ${BASE_REGISTRY:-public.ecr.aws/docker/library} }`.
CI sets `env: BASE_REGISTRY: ghcr.io/thin-edge/tedge-dot/mirror` at workflow level in `ci.yaml`
and `cloud-e2e.yaml`; `publish-simulators.yaml` passes it as a `build-args` entry.
Unset (local), nothing changes.

*Alternative:* a separate `BROKER_IMAGE` variable. Rejected — two knobs that must agree.

### 4. Pre-pull with fallback, retag to the expected reference

`_pull-stack-images` currently greps literal `image:` lines, which breaks once they are
interpolated. It switches to:

1. `docker compose -f <file> config --images` for runtime images, and the `FROM` lines of every
   Dockerfile the stack's `build:` entries reference (via `docker compose config --format json`)
   for base images, both with `BASE_REGISTRY` resolved.
2. For each registry image whose reference starts with the mirror prefix: try the mirror; on
   failure try `public.ecr.aws/docker/library/<suffix>`, then `docker.io/library/<suffix>`, and
   `docker tag` the result to the mirror reference.
3. Backoff for `toomanyrequests` grows to ~2 min total with jitter; other errors move on to the
   next source immediately.

With the docker (not docker-container) BuildKit driver that the e2e jobs use, a `FROM` whose image
is already in the local store is used without contacting a registry, so after step 2 neither the
compose `up` nor the image builds touch any registry for base images. The same recipe serves
`just sim` (C build & conformance jobs) and `_interop`.

### 5. Log in to GHCR in every image-pulling job

`docker/login-action@v3` with `registry: ghcr.io`, `username: ${{ github.actor }}`,
`password: ${{ secrets.GITHUB_TOKEN }}`; `ci.yaml` permissions gain `packages: read`. The step is
`continue-on-error: true` — public packages also pull anonymously — so a login hiccup cannot fail a
job.

### 6. Lift the `max-parallel: 2` caps after the mirror is proven

The caps exist only because of registry throttling (their comments say so). After two weeks of
green runs on `main` with the mirror, raise them (or drop them), and rewrite the comments and the
`_pull-stack-images` header so they describe the mirror rather than the old ECR workaround.

### 7. Flaky reload test: wait on the condition, then fix the cause

The single non-registry e2e failure (`A Changed Service Name Restarts The Connector`, C build,
modbus, run 38420986393) timed out waiting for `"status":"down"` on the old service's health topic.
First step is to read that run's `output.xml` / connector log to determine whether the C runtime
skipped or delayed the `down` publish on rename (a real defect — fix it in the runtime) or the
suite missed a message that was published before `Clear Messages` took effect (a test race — wait
on the retained state instead). Not a blanket timeout bump.

*Outcome:* a test race. The broker log shows the old session disconnecting the instant the
`sed -i` landed, ~30 ms before `Clear Messages` and the SIGHUP: the config directory is watched
(config-file-watch-reload), so the connector restarted on the edit alone and the subsequent
`Clear Messages` discarded its `down`. Fix: every reload test clears the buffer *before* editing
(4 tests in `reload_e2e.robot`, 5 in `modbus_e2e.robot`), which also stops a spurious watcher
restart from being cleared away before the "never went down" assertions see it.

### 8. Dependabot: hold `preact` at 10

Add an `ignore` entry for `preact` major updates in the `/site` npm config (and in its group) until
`@astrojs/preact` peers on 11, with a comment saying when to remove it.

## Risks / Trade-offs

- [Mirror goes stale or the mirror workflow fails repeatedly] → CI keeps using the last good
  mirror; the workflow fails visibly on `main`. Weekly refresh means base-image security updates
  land in CI up to a week late — acceptable for test-only images.
- [GHCR outage or GHCR starts throttling] → the fallback chain in `_pull-stack-images` returns to
  today's ECR/Docker Hub behaviour.
- [`publish-simulators` uses the docker-container BuildKit driver, which cannot see local images]
  → it gets no fallback, only the mirror. It is a PR check only for PRs that touch simulators, and
  its released images are then built from the same mirrored bases CI tested.
- [Mirror packages accidentally private] → fork PRs fail to pull anonymously; login covers same-repo
  PRs. Verified once in the migration plan; the lint job does not check visibility.
- [A forgotten `public.ecr.aws` literal stays in some compose file] → `check-mirrored-images` also
  fails on any literal `public.ecr.aws/` or `docker.io/` `image:` line.
- [Licensing of redistributed images] → the mirrored images are the Docker Official Images and
  `eclipse-mosquitto` (EPL/EDL), redistributed unmodified with their labels; same as any registry
  mirror.

## Migration Plan

1. Land `mirror-images.yaml` + `.github/mirror-images.txt` alone; run it via `workflow_dispatch`;
   make the `mirror/*` packages public and link them to the repository (one-time, manual).
2. Land the `BASE_REGISTRY` plumbing, the new `_pull-stack-images`, GHCR login and the lint job in
   one PR; its own checks are the first proof.
3. Land the dependabot ignore and the reload-test fix independently.
4. After ~2 weeks of green `main` runs, raise `max-parallel`.

Rollback: unset `BASE_REGISTRY` in the workflows — everything falls back to the current upstream
defaults with no other change.

## Open Questions

- Should the mirror workflow pin and record digests (writing them back to the list file) so CI
  is reproducible across the weekly refresh? Deferred; see Non-Goals.
- Is it worth mirroring `ghcr.io/thin-edge/tedge-demo-main-systemd` too? It is already GHCR, so
  probably not.
