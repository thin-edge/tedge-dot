## Context

The documentation lives in the repository as Markdown: `README.md` (~350 lines), `doc/` (contract,
connector specs, SDK, RFCs, migration, `install.md`, topic guides such as
`reducing-data-volume.md` and `mapping-values.md`), `flows/README.md`, `connectors/README.md`,
`demo/README.md` and `impl/c/README.md`. Much of it is contributor-facing: specs and RFCs. GitHub
renders it, but there is no navigation, search or task-oriented entry point. GitHub Pages is not
enabled for `thin-edge/tedge-dot` yet.

Things the site can build on:

- `doc/contract/schemas/config.schema.json` and `point-library.schema.json` describe a connector
  config and a point library.
- `doc/contract/test-vectors/report/vectors.json` and `.../map/` pin the `report` and `map`
  semantics. Both SDKs already run them.
- `demo/` has a Docker/Podman compose stack with simulators for every protocol, and
  `demo/config/*.toml` is pre-wired to it.
- `changes/*.md` holds per-release highlights (added by `docs/slim-release-notes`).

Constraints: OSS-friendly licences only (MIT/Apache/BSD-style), no runtime backend, and the
site must not become a second copy of the docs that drifts.

## Goals / Non-Goals

**Goals:**

- A fast, accessible, good-looking static site at `https://thin-edge.github.io/tedge-dot/`,
  in light and dark, usable on a phone.
- Task-first information architecture: Get started → Guides (scenarios) → Tools → Reference →
  What's new.
- Each fact has one source. Reference pages are rendered from the files in `doc/`, and config
  snippets in the guides are included from real files (`demo/config/*.toml`,
  `packaging/config/*.toml`), never pasted.
- The interactive tools are correct by construction: they are checked in CI against the same
  schemas and test vectors that the SDKs use.

**Non-Goals:**

- Versioned docs (one site tracks `main`; a release links its tag for raw files).
- A custom domain, i18n, analytics, comments, or any server-side component.
- Rewriting specs or RFCs. They are published as they are, under Reference → Design.

## Decisions

### Static site generator: Astro + Starlight

Starlight (MIT) gives a polished docs theme out of the box: sidebar, search (Pagefind, fully
static), light/dark, accessible components, and it is fast because it ships no JS by default.
Astro islands let the interactive tools be real components on exactly the pages that need them,
and custom pages (the landing page) are free-form Astro.

Alternatives considered:
- *MkDocs Material*: excellent, but customising it beyond the theme (landing page, interactive
  islands) fights the tool, and the project is moving to a successor (Zensical).
- *Docusaurus*: capable, but heavier (React everywhere) and slower to load.
- *VitePress*: light and fast, but Vue-only islands and a less complete docs theme.

Interactive components are written in **Preact** (MIT, ~4 kB) as Astro islands.

### Single source: a sync step, not copies

A Node script, `site/scripts/sync-docs.mjs`, runs before every build and dev server. It reads
the page map `site/docs-map.yaml`, which lists each repository file that appears on the site,
its slug and its sidebar section, and writes generated pages into
`site/src/content/docs/reference/` (git-ignored). While doing so it:

- adds Starlight front matter: `title` from the file's first `#` heading (removed from the
  body), plus `description` and an "Edit on GitHub" link to the source file;
- rewrites relative links: a link to a mapped file becomes a site link, and a link to anything
  else (code, configs, unmapped docs) becomes a GitHub `blob/main` URL;
- copies referenced images.

Files not in the map (e.g. `doc/upstream/*`, flow `TEST.md`s) stay GitHub-only. The map is the
one place that decides what is published. Guides and the landing page are authored in `site/`,
since they are new content with no repository counterpart.

Alternative: point Astro's content `glob` loader at `../doc` directly. This avoids the generated
copy, but it cannot rewrite links into code outside the site or add the title front matter,
and Starlight's docs collection expects its own loader. The sync script is about 150 lines and
easy to test.

### Snippets included from real files

Guides import configs with Vite's `?raw` (e.g. `import cfg from '../../../demo/config/modbus.toml?raw'`)
and render them with Starlight's `<Code>` component, which can mark lines. A demo config that
changes changes the guide. A guide that needs a fragment selects it by marker comments in the
source file, not by line numbers.

### Interactive tools

- **Config builder**: a form (protocol, `[connector]`, one or more `[[device]]`, points) driven
  by `config.schema.json` plus small per-protocol shapes (`site/src/data/protocols.json`: the
  types of the `protocol_address`, `address` and `connection` keys the connector specs define,
  other keys allowed), because the contract schema leaves those tables open. It emits TOML with `smol-toml` (MIT) and validates it live with
  `ajv` (MIT). It can load any `demo/config/*.toml` as a starting point, and offers copy and
  download. CI checks that every config in `demo/config` and `packaging/config` validates and
  round-trips through the builder's parse → emit path.
- **Package picker**: a few questions (architecture, glibc / distro age, PROFIBUS, OPC UA
  structures or other Rust-only features, footprint). The answer is computed from a small data
  table, `site/src/data/packages.yaml`, which the install guide also renders, so the table and
  the picker agree.
- **Report-policy playground**: draw or pick a signal (sine, step, noisy, flat), set the
  `report` table, and watch the published samples, heartbeats and held-back changes on a chart.
  The engine is a TypeScript port of the policy. **It runs every case in
  `doc/contract/test-vectors/report/vectors.json` in CI**, as the Rust and C SDKs do, so it cannot
  drift unnoticed.
  Alternative: compile `impl/rust/crates/sdk/src/report.rs` to WASM. That would be exact by
  construction, but it puts a Rust → WASM toolchain into the site build for one small state
  machine. The vectors give the same guarantee more cheaply. Revisit if more SDK logic moves
  into the browser.

### Landing page with wow-factor, done responsibly

A custom Astro page with:
- a hero whose animated SVG shows samples flowing from device icons (Modbus, OPC UA, CAN, SNMP,
  PROFIBUS) through `tedge-dot` and the flows into thin-edge.io and the cloud;
- a protocol grid, a "two packages, one contract" comparison, a terminal-style "try it in a
  minute" block (`docker compose up` with the demo), and cards linking to the scenario guides.

Constraints that keep it professional: pure SVG/CSS animation (no video, no canvas libraries),
`prefers-reduced-motion` respected, under 100 kB of JS on the landing page, Lighthouse
performance and accessibility ≥ 95, and theme tokens shared with Starlight so that light and
dark mode both work.

### What's new from `changes/`

At build time the What's new page reads `changes/*.md` and asks git which release first contained
each fragment: the commit that added the file, then the lowest semver tag containing it.
Fragments not yet in a tag go under "Unreleased". Releases are newest first, and each links its
GitHub release. The deploy job checks out with `fetch-depth: 0`. This is the same selection the
release workflow makes, so the GitHub release and the site always agree.

### Build and deploy

- `site/` is a standalone npm project (`package-lock.json` committed, Node LTS).
- Workflow `docs.yaml`:
  - on pull requests touching `doc/**`, `changes/**`, `site/**`, `README.md`, `demo/config/**`,
    `packaging/config/**` or the contract: sync, build, check internal links
    (`starlight-links-validator`), run the tool tests (vectors, config round-trip);
  - on push to `main`, on `release: published` and manually: the same, then
    `actions/upload-pages-artifact` + `actions/deploy-pages`.
- The deploy job (and the Pages artifact upload) run only when the repository variable
  `DOCS_PAGES` is `"true"`, so `main` stays green before Pages is enabled (Migration Plan step 1).
- `astro.config` sets `site: https://thin-edge.github.io`, `base: /tedge-dot`.
- `just docs` / `just docs-build` wrap the npm commands for contributors.

## Risks / Trade-offs

- [Rendered specs read oddly out of GitHub context (relative links, GitHub-only Markdown such
  as `> [!NOTE]`)] → the sync script maps GitHub alerts to Starlight asides, and the link
  validator fails the build on anything it leaves broken.
- [Guides go stale against the product] → configs are included from real files; each guide
  names the demo service it uses, and a task adds a CI smoke that runs one guide's commands
  against the compose stack (Modbus first).
- [TypeScript report engine differs from the SDKs] → the vectors in CI; a new vector added for
  an SDK change fails the site build until the engine is updated.
- [The schema does not capture everything the SDK validates] → the config builder states that
  it checks structure and points to `tedge-dot describe` / a dry run for full validation.
- [The landing animation looks gimmicky or hurts performance] → the budget and reduced-motion
  rules above; it is reviewed in both themes and on mobile before release.
- [Node toolchain in a Rust/C repo] → isolated in `site/`; it is never part of package builds,
  and Dependabot covers `site/package.json`.

## Migration Plan

1. Land the site behind no switch (Pages not yet enabled): the build and PR checks run, nothing
   is published.
2. Enable GitHub Pages with the "GitHub Actions" source (a repository admin action), then run
   the deploy workflow.
3. Point `packaging/release-notes.md`'s install/README links and the README's top section at the
   site; set the repository's homepage URL.
Rollback: disable Pages or revert the workflow. The repository docs remain complete on their
own throughout.

## Open Questions

- Brand: reuse thin-edge.io's colours and logo, or give tedge-dot its own mark? (Default: the
  thin-edge.io palette with a tedge-dot wordmark, pending a maintainer's OK.)
- Should the site live under the thin-edge.io docs (`thin-edge.github.io/thin-edge.io/`) later,
  rather than a separate project site? This design keeps the content portable (plain Markdown
  plus a few islands) so either works.
- Recorded terminal sessions (asciinema, Apache-2.0 player) for the guides: worth the effort,
  or are static code blocks enough?
