# Spec Delta

## Purpose

Defines the Device Management plugin that keeps connector files in the configuration repository,
edits them in one place, and installs them on gateways.

## ADDED Requirements

### Requirement: OT connectors page
The plugin SHALL add "OT connectors" to the Management menu, after "Configuration repository". The
page SHALL list the configuration repository entries whose configuration type ends with
`::tedge-dot`, and no other entries. Each row SHALL show the entry's name, description, file name
(the type without the suffix), device type and last update.

#### Scenario: Mixed repository
- **WHEN** the repository holds entries of the types `opcua-pump::tedge-dot`, `tedge.toml` and `mosquitto.conf`
- **THEN** the page lists only the `opcua-pump::tedge-dot` entry

### Requirement: Create and edit entries
Users SHALL be able to add an entry with a name, a description, a file name, an optional device
type and TOML content, and to edit an entry's content in a code editor with TOML highlighting.
Saving SHALL first check the TOML syntax and refuse invalid content with the line and column of the
error. The file name SHALL follow the plugin's name rule. A saved entry SHALL have the
configuration type `<file name>::tedge-dot` and point to an uploaded binary with the content.

#### Scenario: Syntax error
- **WHEN** a user saves content with an unclosed string on line 12
- **THEN** the editor shows the error at line 12 and nothing is uploaded

#### Scenario: New entry
- **WHEN** a user adds the file name `line-3` with valid TOML
- **THEN** the repository holds an entry of type `line-3::tedge-dot` whose binary is that content

### Requirement: Delete entries
Deleting an entry SHALL ask for confirmation, then remove the entry and its binary.

#### Scenario: Confirmed delete
- **WHEN** a user confirms deleting an entry
- **THEN** the entry and its binary are gone from the repository

### Requirement: Send entries to gateways
Users SHALL be able to send an entry to one gateway, as a `c8y_DownloadConfigFile` operation with
the entry's type and binary URL, or to a group of gateways, as a bulk operation with the same
operation. After sending, the page SHALL link to the created operation or bulk operation.

#### Scenario: Send to a group
- **WHEN** a user sends an entry to a group of 12 gateways
- **THEN** one bulk operation is created whose operation installs that entry's type and binary

### Requirement: OT connectors device tab
Gateways whose `c8y_SupportedConfigurations` contains a type ending in `::tedge-dot` SHALL get an
"OT connectors" tab. Other devices SHALL not get it. The tab SHALL list the gateway's connector
files and, for the selected file, request a snapshot (`c8y_UploadConfigFile`), show the latest
snapshot with its time, save it to the repository as a new entry, and download it. The tab SHALL
also list the repository's connector entries and send a selected entry to this gateway, including
an entry for a file the gateway does not have yet.

#### Scenario: Gateway without connector types
- **WHEN** a device's supported configurations contain no `::tedge-dot` type
- **THEN** the device has no "OT connectors" tab

#### Scenario: Snapshot to repository
- **WHEN** a user requests a snapshot of `opcua-pump` and saves it to the repository
- **THEN** a new entry of type `opcua-pump::tedge-dot` holds the snapshot's content

#### Scenario: Adding a file to a gateway
- **WHEN** a user sends the entry `line-3::tedge-dot` to a gateway that has no `line-3` file
- **THEN** a `c8y_DownloadConfigFile` operation with that type is created for the gateway
