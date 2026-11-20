## 1. Project

- [ ] 1.1 Scaffold the Web SDK 1025 plugin project in `ui/`: `package.json` with the `c8y.application` manifest exporting the plugin module, lockfile, `angular.json` with the devkit builders, Jest, lint, and a dev server against Device Management
- [ ] 1.2 Check the 1025 typings for `RepositoryService`, the configuration components, `EditorComponent`, `OperationService` and `OperationBulkService`, and adjust the design if they changed

## 2. Validation

- [ ] 2.1 Build step that brings the config builder's validation module and its schema and protocol files into `ui/`, with a test that the UI and the config builder give the same verdicts
- [ ] 2.2 TOML grammar for Monaco, syntax errors as editor markers, and schema errors listed with their key

## 3. OT connectors page

- [ ] 3.1 Navigator node under Management and the list of `tedge-dot-` entries
- [ ] 3.2 Add and edit: file name rule, editor, validation, save through `RepositoryService`
- [ ] 3.3 Delete with confirmation
- [ ] 3.4 Send to a gateway, registering the file first when needed, and to a group with the pre-check

## 4. Registration

- [ ] 4.1 Registration service: fresh snapshot, append-only edit, conflict check, confirmation, send, wait for the new type, temporary binary cleanup
- [ ] 4.2 Unit tests for the text edit (byte-for-byte preservation, conflicts) and the operation sequence

## 5. Device tab

- [ ] 5.1 Tab for devices with `tedge-configuration-plugin`, using Device Management's configuration components limited to `tedge-dot-` types
- [ ] 5.2 "Add connector file", registering a file already on the gateway, and opening an entry on the OT connectors page

## 6. CI and release

- [ ] 6.1 CI job for `ui/`: install, lint, test, build
- [ ] 6.2 Release asset: the plugin zip

## 7. Docs

- [ ] 7.1 Guide for managing connector files from Cumulocity: adding the plugin to Device Management, type names, registration, what validation covers and what it cannot see, minimum tedge-dot version; `doc/install.md` and the README
- [ ] 7.2 Docs site: place the new doc in `site/docs-map.yaml` and update the guides that cover configuration delivery
- [ ] 7.3 Release highlight in `changes/c8y-connector-files.md`

## 8. Verification

- [ ] 8.1 On a test tenant with a thin-edge.io gateway, install the plugin, run the spec's scenarios, and record the results in the PR
