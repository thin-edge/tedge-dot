## ADDED Requirements

### Requirement: Repository-owned mirror of third-party CI images
The repository SHALL maintain a mirror of every third-party container image used by its
Dockerfiles and compose stacks under `ghcr.io/thin-edge/tedge-dot/mirror/<name>:<tag>`, populated
by a workflow that runs on a weekly schedule, on manual dispatch, and on pushes to `main` that change
the mirrored-image list. Mirrored images MUST keep every platform of the upstream multi-arch index,
and the mirror packages MUST be publicly pullable.

#### Scenario: Weekly refresh
- **WHEN** the mirror workflow runs on its schedule
- **THEN** each entry of `.github/mirror-images.txt` is copied from upstream to its mirror reference with all upstream platforms (at least linux/amd64 and linux/arm64)

#### Scenario: Upstream unavailable during refresh
- **WHEN** copying an entry fails after its retries (e.g. upstream returns 429)
- **THEN** the previous mirror tag is left untouched, the remaining entries are still processed, and the workflow run fails visibly

#### Scenario: New image added to the list
- **WHEN** a commit on `main` adds an entry to `.github/mirror-images.txt`
- **THEN** the mirror workflow runs and publishes that entry without a manual trigger

### Requirement: Every third-party image is listed for mirroring
A CI lint job SHALL fail when any Dockerfile `FROM ${BASE_REGISTRY}/<image>` or compose
`${BASE_REGISTRY:-…}/<image>` reference names an image absent from `.github/mirror-images.txt`, or
when a compose file references `public.ecr.aws/` or `docker.io/` literally.

#### Scenario: Unlisted base image
- **WHEN** a PR adds `FROM ${BASE_REGISTRY}/python:3.13-slim` and the list has no `python:3.13-slim`
- **THEN** the `check-mirrored-images` job fails and names the missing entry and the file referencing it

#### Scenario: Hard-coded upstream registry in compose
- **WHEN** a compose file contains `image: public.ecr.aws/docker/library/eclipse-mosquitto:2`
- **THEN** the `check-mirrored-images` job fails

#### Scenario: Consistent references
- **WHEN** every referenced image is listed and no literal upstream registry appears
- **THEN** the job passes

### Requirement: BASE_REGISTRY selects the registry for builds and stacks
Every Dockerfile and every compose registry image SHALL resolve third-party images as
`<BASE_REGISTRY>/<name>:<tag>`, with compose passing `BASE_REGISTRY` to each `build:` as a build
argument. When `BASE_REGISTRY` is unset the default SHALL be `public.ecr.aws/docker/library`.
`ci.yaml`, `cloud-e2e.yaml` and `publish-simulators.yaml` SHALL set it to the mirror.

#### Scenario: Local run without configuration
- **WHEN** a developer runs `just test-e2e modbus` without setting `BASE_REGISTRY`
- **THEN** the broker and all base images resolve under `public.ecr.aws/docker/library`, and no GHCR login is needed

#### Scenario: CI run
- **WHEN** any job of `ci.yaml` builds an image or starts a stack
- **THEN** every third-party image it uses resolves under `ghcr.io/thin-edge/tedge-dot/mirror`

### Requirement: Image pre-pull falls back to upstream registries
Before a stack is built or started, the pre-pull step SHALL fetch every registry image the stack
needs — runtime `image:` references and the `FROM` images of its built services — so that the
subsequent builds and `compose up` do not contact a registry for them. For a reference under the
mirror it SHALL try the mirror, then `public.ecr.aws/docker/library`, then `docker.io/library`, and
tag a successful fallback pull with the mirror reference. A `toomanyrequests` response SHALL be
retried with jittered backoff totalling at least two minutes per source; other errors SHALL move to
the next source immediately. The step SHALL remain best effort: if all sources fail it warns and
lets compose report the error in context.

#### Scenario: Mirror available
- **WHEN** CI pre-pulls the modbus stack and GHCR serves every mirror image
- **THEN** no request is made to `public.ecr.aws` or `docker.io`

#### Scenario: Mirror missing an image
- **WHEN** the mirror lacks `python:3.11` but ECR Public serves it
- **THEN** the image is pulled from ECR Public, tagged as `ghcr.io/thin-edge/tedge-dot/mirror/python:3.11`, and the simulator build uses the local image

#### Scenario: Throttled source
- **WHEN** a source answers `429 toomanyrequests`
- **THEN** the pull is retried with growing, jittered delays before the next source is tried

### Requirement: Image-pulling CI jobs authenticate to GHCR without failing on login
Every CI job that pulls or builds container images SHALL log in to `ghcr.io` with the workflow's
`GITHUB_TOKEN` (workflow permission `packages: read`), and a failed login SHALL NOT fail the job.

#### Scenario: Fork pull request
- **WHEN** CI runs for a pull request from a fork
- **THEN** mirror images are pulled (authenticated with the read-only token, or anonymously if login fails) and the job proceeds to its tests

#### Scenario: Dependabot pull request
- **WHEN** CI runs for a dependabot pull request
- **THEN** mirror images are pulled the same way and no repository secret is required

### Requirement: Dependabot site updates stay installable
The dependabot configuration for `/site` SHALL NOT propose dependency versions that `npm ci`
cannot resolve against the site's current framework peers; specifically `preact` major updates
SHALL be ignored while `@astrojs/preact` requires `preact@^10`.

#### Scenario: Weekly site update
- **WHEN** dependabot opens its weekly `/site` group PR
- **THEN** the Docs workflow's install step succeeds (no `ERESOLVE` on `preact`)
