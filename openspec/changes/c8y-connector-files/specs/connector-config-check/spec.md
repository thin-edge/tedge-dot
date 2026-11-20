# Spec Delta

## Purpose

Defines `tedge-dot check`, which tells whether a connector file is usable before anything
installs it, and whether a remote sender may install it.

## ADDED Requirements

### Requirement: Check a file without connecting
`tedge-dot check <file>` SHALL load the file with the loader the `run` service uses, resolve its
point libraries, and run its protocol module's configuration checks, without connecting to any
device or to the MQTT broker. It SHALL exit 0 when the service would accept the file. Otherwise it
SHALL exit 1 and print one line to stderr that names the file and the problem. Both builds SHALL
give the same verdict and the same message for the same file.

#### Scenario: Usable file
- **WHEN** the file is a valid OPC UA connector configuration whose endpoint is unreachable
- **THEN** the command exits 0 without trying to connect

#### Scenario: Unusable file
- **WHEN** a point declares `datatype = "float33"`
- **THEN** the command exits 1 with the same message the service logs when it refuses that file

#### Scenario: Missing point library
- **WHEN** a device names a point library in `points_from` that no library directory holds
- **THEN** the command exits 1 and names the library

### Requirement: Check a file as if it were installed
`--as <path>` SHALL make the check treat the file as if it were at `<path>`: relative point
library references and relative `point_library_path` entries resolve against the directory of
`<path>`, and messages name `<path>`. Without `--as`, the file's own path SHALL be used.

#### Scenario: Downloaded file
- **WHEN** a file downloaded to a temporary directory names `points_from = ["./points.d/pump.toml"]` and is checked with `--as /etc/tedge/plugins/ot/pump.toml`
- **THEN** the library is looked up as `/etc/tedge/plugins/ot/points.d/pump.toml`

### Requirement: Remote-change rules
With `--remote`, the check SHALL also refuse what a management command may not do, comparing the
file with the file currently at the `--as` path. That means:
- a local-only setting of the file's protocol that the file adds or changes;
- a path reference in `points_from` that the installed file does not have;
- a changed `point_library_path`.

When no file is at that path, every such setting and reference SHALL count as added. Settings
the file keeps unchanged, or removes, SHALL be accepted. The message SHALL name the setting and
where it is (the section, or the device by name).

#### Scenario: Changed endpoint
- **WHEN** the file differs from the installed one only in a device's `endpoint`
- **THEN** the check with `--remote` exits 0

#### Scenario: Added password file
- **WHEN** the file adds `password_file = "/etc/tedge/device-certs/tedge-private-key.pem"` to a device that had none
- **THEN** the check with `--remote` exits 1 and names `password_file` and the device

#### Scenario: Unchanged password file
- **WHEN** the installed file and the new file both set the same `password_file` for a device
- **THEN** the check with `--remote` accepts that setting

#### Scenario: New file with a local-only setting
- **WHEN** no file exists at the `--as` path and the file sets `trust_any_server_certificate = true`
- **THEN** the check with `--remote` exits 1 and names `trust_any_server_certificate`
