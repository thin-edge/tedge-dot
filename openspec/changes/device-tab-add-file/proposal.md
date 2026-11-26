# Proposal

## Why

On a gateway's OT connectors tab, **Register a file on the gateway** is the only action that takes
a new file name. Registering only appends the path to the gateway's list of managed files, so a
user who wants a new connector file ends up with a registered name and no file behind it: no
repository entry to send, and a snapshot that fails with "Config file not found"
(thin-edge/tedge-dot#139). **Add connector file** on the tab lists only entries that are already in
the repository (thin-edge/tedge-dot#140). The register dialog also can't register a point library
that is already on the gateway (thin-edge/tedge-dot#141). A new connector file keeps its template's
service name, `tedge-dot-<protocol>`, which the packaged example file of that protocol also uses;
sent to a gateway with that example, the two connectors keep knocking each other off the broker
(thin-edge/tedge-dot#142). The tab's new flow sends a new file right after saving, which makes that
more likely.

## What Changes

- **One Add button.** The tab's **Add connector file** starts with a choice: *From the
  repository* (sends an entry, as today), *New file* (creates an entry, then sends it to this
  gateway) and *Already on the gateway* (registers only, and says that it doesn't create or send
  the file). The separate **Register a file on the gateway** button goes away.
- **Register point libraries.** *Already on the gateway* registers a connector file or a point
  library; for a library, the protocol and the name, with the resulting path.
- **Create from the tab.** *New file*, and **Create connector file** (or **Create point library**)
  for a selected type without a repository entry, open the editor for this gateway. For a selected
  type, the file name, and a library's protocol, are fixed. After saving, the send dialog opens for
  the gateway; Save, Cancel and the send dialog return to the tab. **Edit in repository** from the
  tab returns there too.
- **Registered but missing.** When the newest snapshot request for the selected type failed
  because the file isn't on the gateway, the tab says so and offers to create or send it.
- Creating needs the roles to add repository entries, as Add, Upload and Duplicate on the OT
  connectors page do.
- **Service name of a new file.** A new connector file's service name follows its file name,
  `tedge-dot-<file name>`, until the user sets another one.
- **Packaged service names.** The editor warns, without blocking Save, when a connector file uses
  the service name of an example file the packages install (such as `tedge-dot-opcua`), and offers
  `tedge-dot-<file name>` instead. Warnings about clashes with other repository entries or with
  files on a gateway, also proposed in #142, are not part of this change.

## Impact

- `ui/` only: the device tab, the send dialog, the editor, and small tested helpers in
  `ui/src/shared/`. Nothing changes on gateways.
- Docs: the OT connectors tab section of `doc/c8y-connector-files.md`; a release highlight.
