## 1. Mirror workflow

- [x] 1.1 Add `.github/mirror-images.txt` listing every third-party image in use: `rust:1-bookworm`, `debian:bookworm`, `debian:bookworm-slim`, `python:3.11`, `python:3.11-slim`, `python:3.11-slim-bookworm`, `python:3.12-slim`, `eclipse-mosquitto:2`, `alpine:3` (re-derive with a grep over Dockerfiles/compose files before committing)
- [x] 1.2 Add `.github/workflows/mirror-images.yaml` (weekly cron, `workflow_dispatch`, push to `main` on the list file; `packages: write`): log in to GHCR, `docker buildx imagetools create` each entry from ECR Public with Docker Hub fallback, long jittered retry on 429, continue past failures and fail the run at the end if any entry failed
- [ ] 1.3 Run it via `workflow_dispatch`; make the `mirror/*` GHCR packages public and linked to the repo; verify `docker manifest inspect ghcr.io/thin-edge/tedge-dot/mirror/eclipse-mosquitto:2` shows amd64 + arm64 from a logged-out shell

## 2. BASE_REGISTRY plumbing

- [x] 2.1 Replace every literal `public.ecr.aws/docker/library/<img>` `image:` in compose files (all `connectors/*/docker-compose*.yaml`, `connectors/opcua/interop/docker-compose.yaml`, `cloud/*/docker-compose.yaml` if any) with `${BASE_REGISTRY:-public.ecr.aws/docker/library}/<img>`, and update the interop file's comment about `_pull-stack-images` grepping `image:`
- [x] 2.2 Add `BASE_REGISTRY: ${BASE_REGISTRY:-public.ecr.aws/docker/library}` to the `args:` of every `build:` in those compose files (connector, simulator, flows, snmp agent/sim, cloud tedge)
- [x] 2.3 Update the Dockerfile header comments ("override BASE_REGISTRY …") to mention the CI mirror; no `FROM` changes
- [x] 2.4 Verify locally that `just test-e2e modbus` with `BASE_REGISTRY` unset still resolves everything from ECR Public, and with `BASE_REGISTRY=ghcr.io/thin-edge/tedge-dot/mirror` from GHCR only (`docker compose config --images`)

## 3. Pre-pull with fallback

- [x] 3.1 Rewrite `_pull-stack-images` to collect runtime images via `docker compose -f <file> config --images` and base images from the `FROM` lines of each built service's Dockerfile (resolved from `docker compose config --format json`, substituting `BASE_REGISTRY`), skipping locally-built tags and stage names
- [x] 3.2 Implement the source chain (mirror → `public.ecr.aws/docker/library` → `docker.io/library`) with `docker tag` back to the mirror reference on fallback; 429 → jittered backoff totalling ≥ 2 min per source, other errors → next source; best effort (warn, never exit non-zero)
- [x] 3.3 Make `just sim` and `_interop` use the same recipe (they already call `_pull-stack-images`; confirm base images of built services are now covered too)
- [x] 3.4 Rewrite the recipe's header comment to describe the mirror and the `Data limit exceeded` quota instead of the old `Rate exceeded` rationale
- [x] 3.5 Test the fallback locally: point `BASE_REGISTRY` at a mirror path with a missing image and confirm the pull falls through to ECR Public, is retagged, and the build does not contact a registry for it

## 4. Workflows

- [x] 4.1 `ci.yaml`: add workflow-level `env: BASE_REGISTRY: ghcr.io/thin-edge/tedge-dot/mirror`, `permissions: packages: read`, and a `docker/login-action@v3` GHCR step (`continue-on-error: true`) in the `c`, `e2e`, `e2e-c` and `interop` jobs
- [x] 4.2 `cloud-e2e.yaml`: same env, permission and login step
- [x] 4.3 `publish-simulators.yaml`: pass `BASE_REGISTRY=ghcr.io/thin-edge/tedge-dot/mirror` via `build-args` for the simulator builds, and log in to GHCR on PRs too (read-only)
- [x] 4.4 Add `just check-mirrored-images` (fails on unlisted `FROM ${BASE_REGISTRY}/…` / `${BASE_REGISTRY:-…}/…` images and on literal `public.ecr.aws/` or `docker.io/` in compose `image:` lines, naming file and image) and a `mirrored-images` job in `ci.yaml` next to `capability-tags`
- [ ] 4.5 Confirm in the PR's own CI logs that the e2e/C jobs pull only from `ghcr.io` (no `public.ecr.aws` lines in the pre-pull or build output)

## 5. Non-registry failures

- [x] 5.1 Download the robot output of run 38420986393 (`robot-output-c-modbus`) and the connector log; determine whether the C runtime skipped/delayed the old service's `down` health message on a service-name change, or the test raced `Clear Messages`
  - Finding: a test race. The broker log shows `tedge-dot-6-modbus` disconnecting at 15:24:06.1205, the moment the `sed -i` landed and before `Clear Messages`/SIGHUP (.154): the watched config directory made the connector restart on the edit itself, and the following `Clear Messages` threw the `down` away. The same edit→clear order was in 3 more reload_e2e tests and 5 modbus_e2e reload tests.
- [x] 5.2 Fix accordingly — in the C runtime if the `down` publish is missing/late (with a Rust/C parity check), otherwise make `A Changed Service Name Restarts The Connector` wait on the retained health state; run the reload suite repeatedly (e.g. 10×) for both implementations to show it is stable
- [x] 5.3 `.github/dependabot.yml`: ignore `preact` semver-major updates for `/site` with a comment to remove once `@astrojs/preact` peers on `preact@11`; close or rebase the current failing dependabot site PR
  - Config done; PR #79 still needs `@dependabot recreate` once this is on `main`.

## 6. Follow-up after the mirror is proven

- [ ] 6.1 After ~2 weeks of green `main` runs, raise `max-parallel` on the `c`, `conformance`, `e2e` and `e2e-c` matrices (or remove it) and rewrite their comments, which currently justify the cap with registry throttling
- [ ] 6.2 Record the outcome (failure rate before/after, any fallback hits seen in logs) in the change's notes or the PR description; no release highlight — the change is CI-only and not user-visible
