# Managing connector files from Cumulocity

The Device Management plugin in [ui/](../ui/) keeps tedge-dot connector files in Cumulocity's
configuration repository, checks and edits them in one place, and installs them on thin-edge.io
gateways through configuration management. Nothing has to be installed or changed on the
gateways.

## Requirements

- Cumulocity with Device Management 1025.
- Gateways with thin-edge.io 2.x and tedge-dot 0.0.13 or later. Earlier tedge-dot releases do not
  apply a changed file on their own; they need a reload after each update, for example the
  `config_update` workflow step the OPC-UA solution blueprint uses.
- The permissions Device Management's configuration repository needs: inventory admin to change
  repository entries, device control to send operations.

## Installing the plugin

1. Download `tedge-dot-ui_<version>.zip` from the
   [release](https://github.com/thin-edge/tedge-dot/releases).
2. In Administration > Ecosystem > Extensions, add the zip.
3. Add the plugin to the Device Management application you use. Plugins go into a custom
   application: if your tenant runs the built-in Device Management, create a copy of it first.

## How files are named

A connector file `/etc/tedge/plugins/ot/<name>.toml` is the configuration type
`tedge-dot-<name>`. Names use letters, digits, `.`, `_` and `-`, and start with a letter or a
digit. The OPC-UA solution blueprint registers `opcua-pump.toml` as `tedge-dot-opcua-pump`, so its
gateways show up as they are.

## OT connectors in the Management menu

The page lists the configuration repository entries whose type starts with `tedge-dot-`.

- **Add connector file** opens the editor with a starting point for a protocol. Give the entry a
  name, the file name it gets on gateways, and optionally a device type.
- **Upload connector files** adds existing `.toml` files. Each file is checked like the editor
  checks a file and becomes an entry named after it: `line-3.toml` is the type `tedge-dot-line-3`.
  Valid files are added in one step, optionally for one device type. A file with problems can be
  opened in the editor to fix it there. Point libraries and other TOML files are refused.
- **Edit** opens the file in the editor. Saving replaces the entry's file.
- **Send to a gateway** installs the file on one gateway, adding it first when the gateway does
  not have it (see [Adding a file to a gateway](#adding-a-file-to-a-gateway)).
- **Send to a group** creates one bulk operation for the devices in a group. A group send never
  adds files: the dialog lists the devices in the group that do not offer the file, because
  their operations would fail. It checks the devices directly in the group, not those in
  subgroups.
- **Delete** removes the entry and its file from the repository. Gateways keep their copy.

### Form and TOML

The editor shows a file as a **form** or as **TOML**, switched above the editing area. Both edit
the same file.

The form has the sections of the docs site's config builder: `[connector]` with its reporting,
grouping and measurement settings, `[mqtt]`, `[connection]`, and each device with its settings,
`protocol_address` and points. Each field has a help text, and a schema error shows under the
field it concerns. *Add device*, *Add point* and the remove buttons start from the protocol's
templates, and switching the protocol gives every device and point the new protocol's address
fields while keeping ids and policies. Keys the form has no field for stay in the file and are
listed as "Also set, kept as is".

A form edit changes only the lines it concerns, so the file keeps its comments and layout. New
devices and points are added at the end of their device's section in the config builder's
layout. Removing a device or point also removes the comments inside its section. If an edit
cannot be written into the file that way, the editor asks before it rewrites the whole file in the
standard layout, and says how many comment lines that drops. Files open in the form, except files
with a syntax error, which open in TOML with the error marked.

### What the editor checks

Before saving, the editor validates the file with the same code as the docs site's config
builder: TOML syntax, then the contract schema (`doc/contract/schemas/config.schema.json`) and
each protocol's address shapes. Syntax errors are marked at their line; schema errors are listed
with the key they concern, for example `device "Pump01" › point "flow" › datatype`.

The check cannot see what only the gateway knows: whether a point library named in `points_from`
exists there, or whether a file named by a setting can be read. **A file that tedge-dot refuses
still counts as installed in Cumulocity**: the connector logs the problem and keeps its previous
configuration until the next restart, after which it stays stopped until the file is fixed. Check
the connector after sending a file that changes library references or file paths.

## The OT connectors tab

Every device that offers the configuration type `tedge-configuration-plugin` gets an **OT
connectors** tab after Configuration: thin-edge.io gateways, and child devices that run their own
thin-edge.io agent. It works like the Configuration tab, limited to connector files:

- **On device** lists the gateway's connector files. *Get snapshot from device* reads the file;
  the preview can save the snapshot to the repository or download it.
- **Available from repository** lists the repository entries of the selected file's type. Unlike
  the Configuration tab, it leaves out entries without a type, which would replace the connector
  file with unrelated content. *Send configuration to device* installs one; *Edit in repository*
  opens it on the OT connectors page.
- **Add connector file** installs a repository entry the gateway does not have yet.
- **Register a file on the gateway** makes a file that is already there, such as a packaged
  `modbus.toml`, a configuration type, so you can take snapshots of it and send new versions.

## Adding a file to a gateway

thin-edge.io's file plugin manages the files listed in
`/etc/tedge/plugins/tedge-configuration-plugin.toml`, and offers that list itself as the type
`tedge-configuration-plugin`. To add the connector file `<name>`, the plugin:

1. requests a snapshot of `tedge-configuration-plugin` from the gateway;
2. stops if the list already has `/etc/tedge/plugins/ot/<name>.toml` under another type, or the
   type `tedge-dot-<name>` for another path;
3. shows the entry it will append and waits for your confirmation:

   ```toml
   # Added by the tedge-dot OT connectors plugin for Device Management
   [[files]]
   path = '/etc/tedge/plugins/ot/<name>.toml'
   type = 'tedge-dot-<name>'
   user = 'tedge'
   group = 'tedge'
   mode = 0o644
   ```

4. sends the extended list back, leaving everything else in it unchanged byte for byte, and
   removes the temporary upload afterwards;
5. waits until the gateway offers `tedge-dot-<name>`, then sends the connector file.

## Moving from a file registered by hand

A connector file already registered with the type `tedge-dot-<name>` needs nothing. One
registered under another type keeps working in the Configuration tab, but this plugin refuses to
register it a second time. Rename its type in `tedge-configuration-plugin.toml` to
`tedge-dot-<name>` to manage it here.
