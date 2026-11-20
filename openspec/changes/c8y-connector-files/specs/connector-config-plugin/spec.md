# Spec Delta

## Purpose

Defines the thin-edge.io configuration plugin that makes connector files configuration types, so
Cumulocity can read, install and add them through configuration management.

## ADDED Requirements

### Requirement: Connector files are configuration types
Both packages SHALL install an executable plugin at `/usr/share/tedge/config-plugins/tedge-dot`.
Its `list` command SHALL print, one per line and sorted, the name of every `*.toml` file directly
in the connector directory (`/etc/tedge/plugins/ot`), without the `.toml` suffix. Files in
subdirectories, such as point libraries in `points.d`, SHALL not be listed. thin-edge.io publishes
each name as the configuration type `<name>::tedge-dot`.

#### Scenario: Packaged files
- **WHEN** the connector directory holds `modbus.toml`, `opcua.toml` and `points.d/modbus/meter.toml`
- **THEN** `list` prints `modbus` and `opcua`, and Cumulocity shows `modbus::tedge-dot` and `opcua::tedge-dot`

### Requirement: Names are file names
Every command SHALL accept only names that match `[A-Za-z0-9][A-Za-z0-9._-]*`, so a name always
means a file directly in the connector directory. Any other name SHALL be refused with a message,
before the command reads or writes a file.

#### Scenario: Path in a name
- **WHEN** an update names the type `../tedge`
- **THEN** the plugin refuses it and no file changes

### Requirement: Snapshots
`get <name>` SHALL write the file's content unchanged to stdout. A name with no file SHALL fail
with a message.

#### Scenario: Snapshot from Cumulocity
- **WHEN** Cumulocity requests the configuration `opcua-pump::tedge-dot`
- **THEN** the snapshot holds the bytes of `/etc/tedge/plugins/ot/opcua-pump.toml`

### Requirement: Updates are checked first
`prepare <name> <file>` SHALL run `tedge-dot check <file> --as <connector directory>/<name>.toml
--remote`. When the check refuses the file, `prepare` SHALL fail with the check's message, and no
file SHALL change. Otherwise it SHALL keep a copy of the new file and of the installed file, if
any, in the work directory.

#### Scenario: Invalid file from Cumulocity
- **WHEN** Cumulocity sends a connector file with a syntax error
- **THEN** the operation fails with the check's message and the connector keeps running with its previous file

#### Scenario: Forbidden change
- **WHEN** Cumulocity sends a connector file that adds `trust_any_server_certificate = true`
- **THEN** the operation fails with a message naming the setting, and the installed file is unchanged

### Requirement: Atomic install
`set <name> <file>` SHALL install the checked file under its name so that the service never reads
a partial file: write it next to the target, then rename it into place. An existing file SHALL keep
its owner and mode. A new file SHALL be owned by `tedge:tedge` with mode `0644`. The running
service then applies it through its configuration watch.

#### Scenario: Changed file
- **WHEN** an update of `opcua-pump::tedge-dot` succeeds
- **THEN** the connector applies the new file within the watch interval, without a restart

### Requirement: New files
When no file has the name, `set` SHALL create it and then ask the agent to refresh its types
(`te/device/main/service/tedge-agent/signal/sync_config`), so the new type appears in Cumulocity.

#### Scenario: Adding a connector file
- **WHEN** Cumulocity sends `line-3::tedge-dot` to a gateway without `line-3.toml`
- **THEN** `/etc/tedge/plugins/ot/line-3.toml` is created, its connector starts, and the gateway's supported configurations include `line-3::tedge-dot`

### Requirement: Verify and roll back
`verify` SHALL succeed only when the installed file is identical to the checked file. `rollback`
SHALL restore the file kept by `prepare`, or remove a file that the update created and refresh the
types.

#### Scenario: Failed update of a new file
- **WHEN** an update that created a file fails after `set`
- **THEN** `rollback` removes the file and the type disappears from the gateway's supported configurations
