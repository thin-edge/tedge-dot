## Context

thin-edge.io's built-in file plugin manages the files listed in
`/etc/tedge/plugins/tedge-configuration-plugin.toml`. Each `[[files]]` entry has a `path`, a
`type`, and the `user`, `group` and `mode` that apply when the plugin creates the file. The plugin
always offers that list itself as the type `tedge-configuration-plugin`, and the agent re-reads the
list when it changes, so Cumulocity can register new files by sending a new list. A file update
succeeds once the file is written. The plugin does no validation and no rollback.

tedge-dot 0.0.13 and later applies a changed file in `/etc/tedge/plugins/ot` within the watch
interval (default 2 s), without a signal. A file it cannot use is logged, and the connector keeps
its previous configuration. After a restart, that connector stays stopped until the file is fixed.

In Cumulocity, a configuration repository entry is a managed object of type
`c8y_ConfigurationDump` with `name`, `description`, `configurationType`, `deviceType` and `url` (a
binary). Device Management offers an entry to devices whose supported configurations contain its
type, and whose type matches its device type when one is set.

The Web SDK (checked against 1024.23.0) provides what the UI needs:
- `RepositoryService` (`@c8y/ngx-components/repository/shared`) lists, creates, updates and
  deletes repository entries and their binaries.
- `DeviceConfigurationListComponent` and `ConfigurationPreviewComponent`
  (`@c8y/ngx-components/repository/configuration`) are the standalone components of Device
  Management's Configuration tab.
- `EditorComponent` (`@c8y/ngx-components/editor`) is a Monaco editor.
- `OperationService` and `OperationBulkService` (`@c8y/client`) create operations and bulk
  operations.

The docs site's config builder (`site/src/lib/config-builder.ts`) already parses TOML and validates
a connector file against `doc/contract/schemas/config.schema.json` and the protocol shapes in
`site/src/data/protocols.json`. Its form (`site/src/components/tools/ConfigBuilder.tsx`, Preact)
edits the parsed object with the module's helpers (`getAt`, `setAt`, `coerce`, `protocolInfo`) and
writes TOML with `emitToml`, which lays a file out the way the shipped configs are and has no
comments. Its field help is in `site/src/lib/help.ts`.

## Goals / Non-Goals

**Goals:**
- One place in Device Management to keep, edit, check and send connector files.
- Writing a connector file in a form, without knowing its TOML keys.
- Point libraries kept, edited and delivered from Cumulocity like connector files.
- A device tab that looks and works like the Configuration tab, limited to connector files.
- Adding a connector file to a gateway without touching the gateway.
- No change to gateways, packages or tedge-dot.

**Non-Goals:**
- Validation with tedge-dot's own loader, rollback, and discovering files nobody registered. A
  gateway-side configuration plugin can add these later.
- Per-gateway values, revisions, a fleet view, secrets, and libraries from Cloud Fieldbus device
  protocols or software packages.

## Decisions

### D1. Cloud only

Everything runs in the browser against Cumulocity's APIs. It works with the gateways and tedge-dot
versions in the field and ships as one plugin zip. The cost is that a file tedge-dot refuses still
counts as installed (see Risks).

### D2. Type names

The connector file `/etc/tedge/plugins/ot/<name>.toml` has the type `tedge-dot-<name>`. Names match
`[A-Za-z0-9][A-Za-z0-9._-]*`, so a name always means a file directly in the connector directory.
The blueprint already registers `opcua-pump.toml` as `tedge-dot-opcua-pump`, so its gateways work
as they are. The UI treats every supported type starting with `tedge-dot-` as a connector file.

### D3. Registering a file from Cumulocity

To add `<name>` to a gateway that does not offer `tedge-dot-<name>`, the UI:
1. requests a snapshot of `tedge-configuration-plugin` and waits for it;
2. refuses if the list already has `/etc/tedge/plugins/ot/<name>.toml` under another type;
3. appends one `[[files]]` block (that path, the type, `tedge`, `tedge`, `0o644`) to the text,
   leaving everything else byte for byte, and shows the block for confirmation;
4. uploads the result as a temporary binary, sends it as `c8y_DownloadConfigFile`, and waits;
5. waits until the gateway's `c8y_SupportedConfigurations` contains the new type;
6. sends the connector file, then deletes the temporary binary.

The same steps without step 6 register a file that is already on the gateway, such as a packaged
`modbus.toml`. Registration runs one gateway at a time. A group send never registers files.

### D4. Validation shared with the config builder

The editor validates with the config builder's code (`parseToml`, `validateConfig`) and the same
schema files. Syntax errors become editor markers at their line. Schema errors are listed with the
key they concern, because the parsed document no longer knows its line numbers. A build step brings the module and its JSON files
into `ui/`, so the repository holds one copy of the rules and the UI follows schema changes. The
check covers syntax, keys, types and protocol shapes. It cannot see what only the gateway knows,
such as which point libraries exist there.

### D5. The device tab reuses Device Management's components

The tab shows on devices whose `c8y_SupportedConfigurations` contains `tedge-configuration-plugin`,
which every thin-edge.io gateway offers. A gateway without connector files shows an empty list with
"Add connector file". The tab uses `DeviceConfigurationListComponent` and
`ConfigurationPreviewComponent`, limited to `tedge-dot-` types, so snapshots, saving to the
repository, downloading and sending look as they do in the Configuration tab. The preview's "Edit
in repository" opens the entry on the OT connectors page.

### D6. The OT connectors page

A Management navigator node after Configuration repository. The page lists `c8y_ConfigurationDump`
entries whose type starts with `tedge-dot-`. It adds, edits and deletes them with
`RepositoryService`, and edits content in `EditorComponent`. Monaco has no TOML mode, so the plugin
registers a small TOML grammar for highlighting.

### D7. Sending

One gateway gets a `c8y_DownloadConfigFile` operation with the entry's type and binary URL, after
registration (D3) if the gateway lacks the type. A group gets a bulk operation with the same
operation. Before sending to a group, the UI counts the gateways in it that do not offer the type,
and lists them: their operations would fail, and each needs "Add connector file" first.

### D8. Web SDK version and project

The plugin targets Web SDK 1025, the current release line and the 2027 LTS. The 1024.23.0
findings above need a re-check against 1025 when the project is set up. Device Management 1025
runs on Angular 22 without zone.js change detection, so component state lives in signals. The
SDK's `getConfigSnapshot` drops the snapshot's event id, so registration reads the latest
configuration event itself to tell a new snapshot from the previous one. The project follows the
devkit's plugin layout: a `c8y.application` manifest in `package.json` exporting one module,
`@c8y/devkit` builders, and Jest for the logic, which stays in plain TypeScript files under
`src/shared/` (including the order of the registration steps, tested with fakes). It lives in
`ui/` with its own lockfile and CI job, like `site/`.

### D9. Form and TOML views of the same file

The editor shows the file in one of two views, switched with a toggle:

- **TOML**: the code editor of D6.
- **Form**: the sections of the docs site's config builder in Angular: `[connector]` with
  `report`, `measurement` and `grouping`, `[mqtt]`, `[connection]`, and each `[[device]]` with its
  settings, `protocol_address` and points. Protocol tables take their fields from the builder's
  protocol shapes. Each field has the builder's help text, and a schema error shows at the field it
  concerns. Devices and points can be added and removed. The protocol can be switched: devices and
  points get the new protocol's address templates, and ids and policies are kept. Keys the form
  does not show are kept as they are and listed.

The file's text is the one source. The form parses it, and a form edit is written into the text,
so comments and layout stay: `@decimalturn/toml-patch` (3.3.2, MIT, no dependencies) applies the
difference between the file and the edited configuration. It misplaces new `[[device]]` and
`[[device.point]]` tables, so the plugin writes new devices and points itself, in `emitToml`'s
layout, at the end of their device's section (a new device after the last device's trailing
comments). toml-patch appends a new key below a table's trailing comments and later treats
comments right above a key as the key's own, so a blank line goes between them. Every result is
parsed again and must equal the edited configuration. When it does not, the whole file is written
with `emitToml`, after a confirmation that names the comment lines it drops. A file with a syntax
error has no form; the notice points to the line in the TOML view.

Files open in the form, except files the form cannot read, which open in TOML.

The form follows the Web SDK's Codex patterns: `c8y-form-group` with `c8y-messages` for each field
and its error, sentence-case labels with the TOML key as the title of the field's help popover,
`c8y-switch` for on/off settings, Codex collapse sections (legend-style toggles) for reporting,
grouping, measurement names, device settings and each point, the grid for field rows,
`btn-dot--danger` buttons to remove devices and points, and alerts for deprecated settings.

The site keeps its Preact form; the plugin gets the same model, options and texts. The option
lists the form needs (outputs, grouping policies, aggregates, report fields) move from the site's
simulator modules into `site/src/lib/form-options.ts`, which imports nothing, so the build step of
D4 can bring it and `help.ts` into `ui/` without the simulator. The plain logic of the form (which
fields a protocol table has, adding and removing devices and points, switching protocols, writing
an edit into the text) lives in `src/shared/` with unit tests, which edit the repository's
commented demo configurations.

### D10. Point libraries as repository entries

A point library (contract §3.4: `[library]` and `[[point]]`, a device type's point list without
connection data) is a repository entry like a connector file. Its configuration type is
`tedge-dot-points.<protocol>.<name>` and its file is
`/etc/tedge/plugins/ot/points.d/<protocol>/<name>.toml`, where `<protocol>` is the library's
`[library] protocol` and `<name>` follows the connector file name rule. Connector file names may
not start with `points.`, so the two kinds never share a type: every `tedge-dot-` type that does
not start with `tedge-dot-points.` is a connector file.

A library is registered (D3) and sent (D7) like a connector file. Its `[[files]]` block also sets
`parent_user = 'tedge'`, `parent_group = 'tedge'` and `parent_mode = 0o755`, because the gateway's
`points.d` holds no protocol folders until the first library arrives, and thin-edge's file plugin
creates the missing folder with those values (thin-edge 2.1). tedge-dot re-reads the libraries its
connector files reference on its own, so a new version of a library applies without a restart.

The tedge-dot packages install libraries too (`demo-sim` per protocol, in
`/usr/share/tedge-dot/points.d`, as `.goreleaser.yaml` lists them). The build step of D4 copies
those files into `ui/`, so the plugin knows them: they count as existing libraries, the form can
show their points, and they are never sent.

### D11. A second list, and the editor for libraries

The OT connectors page gets a second card, Point libraries, below Connector files, with the same
actions: add, upload, edit, delete, send to a gateway and send to a group. Its rows show name,
description, protocol, library name and last update. Deleting a library that connector entries
reference names those entries first.

The editor serves both kinds. For a library it asks for the protocol and the library name instead
of a file name (both fixed once saved, as the file name is), validates with the config builder's
`validateLibrary` and `point-library.schema.json`, and its form shows the `[library]` settings
(type, description, version, and OPC UA's namespaces) and the points, with the same point fields
as a device's. Form edits keep comments as in D9; new points go at the end of the file. Upload
(8.7) adds a library file as a library instead of refusing it.

The device tab lists the gateway's libraries next to its connector files, with their own icon, so
snapshots, saving to the repository and sending work for libraries too, and Add connector file
offers libraries as well.

### D12. References only to libraries that exist

A device's `points_from` may only name a library that exists for the file's protocol: a repository
library or a packaged one (D10). A path, which tedge-dot accepts, is refused, because Cumulocity
can neither deliver nor check the file it names. An unknown reference is a validation problem
like a schema error, so Save stays disabled until it is fixed.

The form's Point libraries field is an ordered list with a dropdown of the libraries available for
the protocol, and buttons to remove and reorder. The form resolves the references with tedge-dot's
rules (§3.4: libraries in order, then the device's own points; a point with an id that exists
patches it, `meta` merged recursively, `map` replaced, other fields replaced) and shows the
inherited points read-only with the library they come from, marks the device's own points that
override one, and shows the device type a device inherits. Resolution is plain TypeScript, tested
with the same cases as `impl/rust/crates/sdk/src/library.rs`; it only drives what the form shows
and the reference check, the gateway resolves on its own.

### D13. Sending a connector file with its libraries

Sending a connector file to a gateway first collects the repository libraries its devices
reference (packaged ones are on every gateway) and the ones the gateway does not offer. Those are
registered together with the connector file in one edit of the list of managed files, with one
confirmation that shows every appended block, and sent before the connector file, which is sent
only after they succeeded. A library the gateway already offers is not sent again; sending the
library itself updates it.

A group send stays one bulk operation without registration (D7). Its pre-check also lists the
gateways that lack a library the file references, and libraries can be sent to a group first.

## Risks / Trade-offs

- [A file tedge-dot refuses counts as installed, and its connector stays stopped after the next
  restart] → Schema validation catches most mistakes before sending. The docs say so plainly, and
  the follow-up gateway plugin closes the gap.
- [A form edit changes or drops a file's comments] → Edits are written into the text and checked
  against the edited values; when that fails, the user is asked before the file is rewritten.
  Removing a device or point removes the comments inside its section, which describe it.
- [The plugin's resolution of library references drifts from tedge-dot's] → It only drives the
  display and the reference check; the tests mirror `library.rs`'s cases, and the gateway resolves
  on its own.
- [A gateway runs another tedge-dot version or a vendor pack, with other packaged libraries] → The
  plugin knows the libraries of its own tedge-dot version; the guide says so, and a site library of
  the same name shadows a packaged one.
- [Every library adds a type to each gateway's list of managed files] → Only libraries a gateway's
  files reference are added, one edit per send.
- [The form and the docs site's builder drift apart] → Both use the same model, options and help
  texts; the site's tests hold the options and texts to the schema.
- [The UI edits thin-edge.io's list of managed files] → It only appends one block for a path in the
  connector directory, shows it before sending, and never edits or removes other entries.
- [The list changes on the gateway between snapshot and send] → The snapshot is taken right before
  the send. The window is a few seconds.
- [Group sends partly fail on gateways without the file] → The pre-check lists them (D7).
- [The SDK APIs differ in 1025] → Checked when the project is set up, before the UI work starts.
- [tedge-dot older than 0.0.13 does not apply changed files on its own] → The docs name the
  minimum version and the blueprint's workflow hook as the workaround.

## Open Questions

- How the UI zip is versioned: with the packages through release-please, or separately.
- How tenants add the plugin to Device Management in 1025 (custom application or the built-in
  one), for the guide.
