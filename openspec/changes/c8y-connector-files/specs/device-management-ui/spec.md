# Spec Delta

## Purpose

Defines the Device Management plugin that keeps connector files in the configuration repository,
checks and edits them in one place, registers them on gateways and installs them, without changes
on the gateways.

## ADDED Requirements

### Requirement: OT connectors page
The plugin SHALL add "OT connectors" to the Management menu, after "Configuration repository". The
page SHALL list the configuration repository entries whose configuration type starts with
`tedge-dot-`, and no other entries. Each row SHALL show the entry's name, description, file name
(the type without the prefix), device type and last update.

#### Scenario: Mixed repository
- **WHEN** the repository holds entries of the types `tedge-dot-opcua-pump`, `tedge.toml` and `mosquitto.conf`
- **THEN** the page lists only the `tedge-dot-opcua-pump` entry

### Requirement: Create and edit entries
Users SHALL be able to add an entry with a name, a description, a file name, an optional device
type and content, and to edit an entry's content in a code editor with TOML highlighting or in a
form. The file
name SHALL match `[A-Za-z0-9][A-Za-z0-9._-]*`. A saved entry SHALL have the configuration type
`tedge-dot-<file name>` and point to an uploaded binary with the content.

#### Scenario: New entry
- **WHEN** a user adds the file name `line-3` with valid content
- **THEN** the repository holds an entry of type `tedge-dot-line-3` whose binary is that content

#### Scenario: Invalid file name
- **WHEN** a user enters the file name `../tedge`
- **THEN** the form refuses it and nothing is saved

### Requirement: Form view
The editor SHALL offer a form view and a TOML view of the same content. The form SHALL cover the
sections of the docs site's config builder: `[connector]` with `report`, `measurement` and
`grouping`, `[mqtt]`, `[connection]`, and each `[[device]]` with its settings, `protocol_address`
and points, with the builder's help text for each field. It SHALL show a schema error at the field
it concerns, add and remove devices and points, and switch the protocol. Keys the form does not
show SHALL be kept. A form edit SHALL change only the text it concerns, keeping the file's comments
and layout. When an edit cannot be written that way, the editor SHALL ask before it rewrites the
whole file in the config builder's layout. Files SHALL open in the form unless the form cannot
read them.

#### Scenario: Editing a field
- **WHEN** a user changes a Modbus device's `port` to `5020` in the form
- **THEN** the TOML view holds `port = 5020` in that device's `protocol_address`

#### Scenario: Error at its field
- **WHEN** a point's `datatype` is `"float33"`
- **THEN** the form shows the schema's message under that point's datatype field

#### Scenario: File with comments
- **WHEN** a user changes a point's `unit` in the form on `opcua-pump.toml`, which has comment lines
- **THEN** only that point's `unit` line changes in the TOML, and every comment line is still there

#### Scenario: New point in a commented file
- **WHEN** a user adds a point to the device of a commented file in the form
- **THEN** the TOML holds a new `[[device.point]]` table at the end of that device's section, and
  every comment line is still there

#### Scenario: Unknown key
- **WHEN** a point has a `meta` table the form has no field for, and the user changes its `unit`
- **THEN** the written file still holds the `meta` table

#### Scenario: Syntax error
- **WHEN** the content has an unclosed string on line 12
- **THEN** the form view shows no form and names line 12

### Requirement: Validation before saving
Before saving, the editor SHALL validate the content with the docs site's config builder
validation and the contract schema. It SHALL mark syntax errors at their line and column, and list
schema errors with the key they concern. Content with errors SHALL not be saved. The UI and the
config builder SHALL give the same verdict for the same content.

#### Scenario: Syntax error
- **WHEN** a user saves content with an unclosed string on line 12
- **THEN** the editor marks line 12 and nothing is uploaded

#### Scenario: Schema error
- **WHEN** the fourth point of the first device declares `datatype = "float33"`
- **THEN** the editor lists the schema's message for that point's `datatype`, and nothing is uploaded

### Requirement: Delete entries
Deleting an entry SHALL ask for confirmation, then remove the entry and its binary.

#### Scenario: Confirmed delete
- **WHEN** a user confirms deleting an entry
- **THEN** the entry and its binary are gone from the repository

### Requirement: Send to a gateway or a group
Users SHALL be able to send an entry to one gateway, as a `c8y_DownloadConfigFile` operation with
the entry's type and binary URL, after registering the file if the gateway lacks the type. They
SHALL also be able to send it to a group, as a bulk operation with the same operation. Before a
group send, the UI SHALL list the gateways in the group that do not offer the type. A group send
SHALL never register files.

#### Scenario: Send to a group
- **WHEN** a user sends an entry to a group of 12 gateways, 3 of which do not offer its type
- **THEN** the UI names those 3 gateways before sending, and one bulk operation is created for the group

### Requirement: Registering a file on a gateway
To register `<name>` on a gateway, the UI SHALL request a fresh snapshot of
`tedge-configuration-plugin`, append one `[[files]]` entry with the path
`/etc/tedge/plugins/ot/<name>.toml`, the type `tedge-dot-<name>`, user and group `tedge` and mode
`0o644`, and send the result back. It SHALL keep the rest of the file unchanged, byte for byte. It
SHALL show the entry for confirmation before sending. It SHALL refuse when the path is already
registered under another type. When the gateway offers the new type, the UI SHALL delete the
temporary binary.

#### Scenario: Adding a connector file
- **WHEN** a user adds the entry `tedge-dot-line-3` to a gateway that does not offer that type
- **THEN** the gateway's list gains one entry for `/etc/tedge/plugins/ot/line-3.toml`, the gateway offers `tedge-dot-line-3`, and a `c8y_DownloadConfigFile` operation installs the file

#### Scenario: Path registered under another type
- **WHEN** the gateway's list already has `/etc/tedge/plugins/ot/line-3.toml` with the type `line3`
- **THEN** the UI refuses to register it, names the existing type, and sends nothing

#### Scenario: File already on the gateway
- **WHEN** a user registers `modbus` on a gateway with the packaged `modbus.toml`
- **THEN** the gateway offers `tedge-dot-modbus`, and its snapshot holds the packaged file

### Requirement: OT connectors device tab
Devices whose `c8y_SupportedConfigurations` contains `tedge-configuration-plugin` SHALL get an "OT
connectors" tab. Other devices SHALL not get it. The tab SHALL list the device's `tedge-dot-` types
with Device Management's configuration components. For the selected file, it SHALL request a
snapshot, show the latest snapshot with its time, save it to the repository, download it, and send
it from a repository entry. The tab SHALL offer "Add connector file" for entries the device does
not offer, and SHALL open an entry on the OT connectors page from the preview.

#### Scenario: Device without thin-edge.io
- **WHEN** a device's supported configurations do not contain `tedge-configuration-plugin`
- **THEN** the device has no "OT connectors" tab

#### Scenario: Gateway without connector files
- **WHEN** a thin-edge.io gateway offers no `tedge-dot-` type
- **THEN** the tab shows an empty list and "Add connector file"

#### Scenario: Snapshot to repository
- **WHEN** a user requests a snapshot of `tedge-dot-opcua-pump` and saves it to the repository
- **THEN** a new entry of type `tedge-dot-opcua-pump` holds the snapshot's content
