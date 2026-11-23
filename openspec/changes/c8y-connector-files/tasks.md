## 1. Project

- [x] 1.1 Scaffold the Web SDK 1025 plugin project in `ui/`: `package.json` with the `c8y.application` manifest exporting the plugin module, lockfile, `angular.json` with the devkit builders, Jest, a type check, and a dev server against Device Management
- [x] 1.2 Check the 1025 typings for `RepositoryService`, the configuration components, `EditorComponent`, `OperationService` and `OperationBulkService`, and adjust the design if they changed

## 2. Validation

- [x] 2.1 Build step that brings the config builder's validation module and its schema and protocol files into `ui/` before every build and test, so the UI runs the config builder's own code; tests run it against the packaged connector files
- [x] 2.2 TOML grammar for Monaco, syntax errors as editor markers, and schema errors listed with their key

## 3. OT connectors page

- [x] 3.1 Navigator node under Management and the list of `tedge-dot-` entries
- [x] 3.2 Add and edit: file name rule, editor, validation, save through `RepositoryService`
- [x] 3.3 Delete with confirmation
- [x] 3.4 Send to a gateway, registering the file first when needed, and to a group with the pre-check

## 4. Registration

- [x] 4.1 Registration service: fresh snapshot, append-only edit, conflict check, confirmation, send, wait for the new type, temporary binary cleanup
- [x] 4.2 Unit tests for the text edit (byte-for-byte preservation, conflicts) and the operation sequence

## 5. Device tab

- [x] 5.1 Tab for devices with `tedge-configuration-plugin`, using Device Management's configuration components limited to `tedge-dot-` types
- [x] 5.2 "Add connector file", registering a file already on the gateway, and opening an entry on the OT connectors page

## 6. CI and release

- [x] 6.1 CI job for `ui/`: install, test, type check, build
- [x] 6.2 Release asset: the plugin zip

## 7. Docs

- [x] 7.1 Guide for managing connector files from Cumulocity: adding the plugin to Device Management, type names, registration, what validation covers and what it cannot see, minimum tedge-dot version; `doc/install.md` and the README
- [x] 7.2 Docs site: place the new doc in `site/docs-map.yaml`; the README and the install guide link it (no site guide covers configuration delivery yet)
- [x] 7.3 Release highlight in `changes/c8y-connector-files.md`

## 8. Form view

- [x] 8.1 Move the form's option lists (outputs, grouping policies, aggregates, report fields) into `site/src/lib/form-options.ts`, re-exported where the site uses them; bring it and `help.ts` into `ui/` with the build step
- [x] 8.2 Form logic in `src/shared/` with unit tests: fields of a protocol table, adding and removing devices and points, switching protocols, the rewrite check and comment count
- [x] 8.3 Form component: the config builder's sections and fields, help texts, errors at their fields, kept keys listed
- [x] 8.4 Form and TOML toggle in the editor: read-only form until the rewrite is accepted, no form on a syntax error, which view a file opens in
- [x] 8.5 Docs: the guide and the release highlight describe the form
- [x] 8.6 Form edits keep comments: write edits into the text with toml-patch, add new devices and points in place, check every result, ask before a full rewrite
- [x] 8.7 Upload an existing `.toml` file on the OT connectors page: check it, add it when valid, open it in the editor otherwise

## 9. Verification

- [ ] 9.1 On a test tenant with a thin-edge.io gateway, install the plugin, run the spec's scenarios, and record the results in the PR
