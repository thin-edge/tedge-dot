## 1. Spike

- [ ] 1.1 On a thin-edge.io 2.1 gateway, confirm that the agent runs a plugin's `prepare` and `set` for a `::tedge-dot` type its `list` does not include. If it does not, switch to the fallback in the design's risks before task 3.

## 2. `tedge-dot check` (both builds)

- [ ] 2.1 Shared test vectors: usable and unusable files with their expected messages, `--as` resolution, and the remote-change cases (changed endpoint; added, unchanged and removed password file; new file with a local-only setting; new `points_from` path; changed `point_library_path`)
- [ ] 2.2 C: the `check` subcommand, reusing the loader and the protocol module's configuration checks without connecting, and the local-only and path-reference rules; tests in `impl/c/tests`
- [ ] 2.3 Rust: the same subcommand, rules and tests
- [ ] 2.4 Both builds pass the vectors with identical verdicts and messages

## 3. Configuration plugin

- [ ] 3.1 `packaging/config-plugin/tedge-dot` (POSIX sh): `list`, `get`, `prepare`, `set`, `verify` and `rollback` as specified, the name rule, the check run as `tedge`, and `sync_config` after a file is added or removed
- [ ] 3.2 Install it in both packages (`.goreleaser.yaml` for tedge-dot-rs, `impl/c/packaging/nfpm.yaml` for tedge-dot-c); `packaging/check-manifest-parity.sh` passes
- [ ] 3.3 Plugin tests with stand-ins for `tedge` and `tedge-dot`
- [ ] 3.4 Cloud e2e test in `cloud/`: snapshot, update, refused update and new file on a thin-edge.io gateway

## 4. Device Management plugin (`ui/`)

- [ ] 4.1 Scaffold the Web SDK 1024 plugin project in `ui/`: `package.json`, lockfile, the devkit configuration that exports the plugin module, lint and unit tests
- [ ] 4.2 OT connectors page: navigator node, list of `::tedge-dot` entries, add and edit with a code editor and a TOML syntax check, delete, send to a gateway or a group
- [ ] 4.3 Device tab: shown for devices with `::tedge-dot` types; files on the device with snapshot, save to repository and download; repository entries with send
- [ ] 4.4 Unit tests for the services and components
- [ ] 4.5 CI job for `ui/` (install, lint, test, build) and the plugin zip as a release asset

## 5. Docs

- [ ] 5.1 Guide for managing connector files from Cumulocity: installing the UI plugin, the plugin's types, the rules for files from the cloud, and moving from a hand-registered file; `doc/install.md` and the README
- [ ] 5.2 Docs site: place the new doc in `site/docs-map.yaml` and update the guides that cover configuration delivery
- [ ] 5.3 Release highlight in `changes/c8y-connector-files.md`

## 6. Verification

- [ ] 6.1 On a test gateway, install both packages and the UI plugin, run the scenarios of the three specs, and record the results in the PR
