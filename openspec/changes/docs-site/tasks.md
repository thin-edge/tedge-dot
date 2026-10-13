## 1. Release highlights groundwork (branch docs/slim-release-notes)

- [x] 1.1 Slim `packaging/release-notes.md` to a fixed header linking the docs at the release tag
- [x] 1.2 Move install, package choice and upgrade notes to `doc/install.md`; link it from the README
- [x] 1.3 Add `changes/` with its README, and select highlights added since the previous tag in `release.yaml`
- [x] 1.4 Add the openspec `tasks` rule that asks for a highlight in `changes/`
- [ ] 1.5 Merge, then regenerate the 0.0.16 release body with the new header (`gh release edit`)

## 2. Site scaffold

- [ ] 2.1 Create `site/` as an Astro + Starlight project (Node LTS, lockfile committed), `base: /tedge-dot`
- [ ] 2.2 Theme: tokens for light/dark, typography, tedge-dot wordmark and favicon; settle the brand open question with a maintainer
- [ ] 2.3 Sidebar skeleton: Get started, Guides, Tools, Reference, What's new
- [ ] 2.4 `just docs` / `just docs-build` recipes; `site/` entries in `.gitignore` (generated content, `dist/`, `node_modules/`)

## 3. Single-source sync

- [ ] 3.1 `site/docs-map.yaml` listing every published repository file, its slug and its section
- [ ] 3.2 `site/scripts/sync-docs.mjs`: front matter from the first heading, "Edit on GitHub" link, image copying
- [ ] 3.3 Link rewriting: mapped files → site links (anchors kept), other paths → GitHub `blob/main`
- [ ] 3.4 GitHub alerts (`> [!NOTE]` etc.) → Starlight asides
- [ ] 3.5 Unit tests for the sync script (link rewriting, title extraction, unmapped files)
- [ ] 3.6 `starlight-links-validator` enabled; fix every broken link it reports in `doc/`

## 4. Get started and guides

- [ ] 4.1 Overview page (what tedge-dot is, architecture diagram, how it fits thin-edge.io)
- [ ] 4.2 Install page rendering `doc/install.md` with the package table from `site/src/data/packages.yaml`
- [ ] 4.3 "Your first device in 5 minutes" with the demo compose stack
- [ ] 4.4 Marker-comment snippet include helper (select part of a repository file by markers)
- [ ] 4.5 Guide: Modbus TCP PLC to Cumulocity
- [ ] 4.6 Guide: secured OPC UA server (PKI, trust, `tedge-dot pki`)
- [ ] 4.7 Guide: CAN bus and CANopen
- [ ] 4.8 Guide: SNMP network devices (poll, SET, traps)
- [ ] 4.9 Guide: a fleet of identical devices with point libraries
- [ ] 4.10 Guide: reducing data volume on a metered link (links the playground)
- [ ] 4.11 Guide: writing to devices from the cloud (device parameters, `map`, literal parameters)
- [ ] 4.12 Guide: small and old gateways with `tedge-dot-c`

## 5. Interactive tools

- [ ] 5.1 Preact integration; shared island styling that follows the site theme
- [ ] 5.2 Config builder: schema-driven form, `smol-toml` emit, `ajv` validation, per-field errors, copy/download
- [ ] 5.3 Config builder: start from any `demo/config/*.toml`
- [ ] 5.4 Check: every `demo/config` and `packaging/config` file validates and round-trips through the builder
- [ ] 5.5 Package picker driven by `packages.yaml`, with an explanation for each recommendation
- [ ] 5.6 Report-policy engine in TypeScript; test that runs every case in `report/vectors.json`
- [ ] 5.7 Report-policy playground UI: signal presets, policy form, chart of readings, published samples and heartbeats

## 6. Landing page

- [ ] 6.1 Hero with animated SVG data flow (devices → tedge-dot → flows → thin-edge.io → cloud), reduced-motion fallback
- [ ] 6.2 Protocol grid, "two packages, one contract" comparison, quick-start terminal block, guide cards
- [ ] 6.3 Review in light/dark and at phone width; JS ≤ 100 kB on the landing page
- [ ] 6.4 Lighthouse CI check (performance and accessibility ≥ 95)

## 7. What's new

- [ ] 7.1 Build-time loader: each `changes/*.md` → the release that first contained it (git), or Unreleased
- [ ] 7.2 What's new page, newest first, linking each GitHub release

## 8. CI and deployment

- [ ] 8.1 `docs.yaml`: PR build + sync tests + link check + tool checks on relevant paths
- [ ] 8.2 Deploy job on push to `main`, `release: published` and dispatch (`fetch-depth: 0`, upload-pages-artifact, deploy-pages)
- [ ] 8.3 Dependabot for `site/package.json`
- [ ] 8.4 Smoke: run the Modbus guide's commands against the demo stack in CI

## 9. Go live

- [ ] 9.1 Ask a repository admin to enable GitHub Pages with the "GitHub Actions" source; run the deploy
- [ ] 9.2 Point `packaging/release-notes.md` and the README's top section at the site; set the repository homepage URL
- [ ] 9.3 Add a release highlight in `changes/docs-site.md` announcing the site
