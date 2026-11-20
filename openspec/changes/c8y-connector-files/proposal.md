## Why

Connector files can reach a gateway from Cumulocity today, but only by hand and without checks:

- A file becomes a configuration type only when someone adds it to
  `/etc/tedge/plugins/tedge-configuration-plugin.toml` on each gateway, as the OPC-UA solution
  blueprint does. A file that is not on the gateway yet cannot be sent at all.
- thin-edge.io's file plugin writes whatever arrives. tedge-dot refuses a management command that
  adds a file reference such as `password_file` or relaxes server authentication, because a remote
  change could otherwise make the gateway send a local file to a server of its choosing. A whole
  file sent through configuration management skips that check, because the reload trusts any file
  on disk.
- Nothing in Cumulocity shows or edits connector files. Users edit TOML locally, upload it to the
  configuration repository and send it per device.

## What Changes

- **`tedge-dot check`.** A new command in both builds validates a candidate connector file with
  the service's own loader and protocol-module checks, without connecting to anything. Given the
  installed file, it also refuses what a management command may not do: new or changed local-only
  settings and new path references.
- **Configuration plugin.** Both packages install a thin-edge.io configuration plugin,
  `/usr/share/tedge/config-plugins/tedge-dot`. It offers every connector file in
  `/etc/tedge/plugins/ot` as the type `<file name>::tedge-dot`, returns it for snapshots, checks a
  sent file with `tedge-dot check` before it touches anything, installs it atomically, adds a file
  that is not on the gateway yet, and restores the previous file when the update fails.
- **Device Management plugin.** A new `ui/` project builds a Cumulocity Web SDK plugin for Device
  Management:
  - Management > OT connectors lists the configuration repository entries for connector files. It
    adds, edits (TOML editor), deletes and sends them to a gateway or a group.
  - An "OT connectors" device tab on gateways with connector types shows the files on the device
    and the matching repository entries, laid out like the Configuration tab.
- **Packaging and CI.** CI builds and tests the UI. Releases attach the plugin zip.

Not in this change: per-gateway values, a structured (form) editor, revisions and fleet views,
point libraries from Cumulocity, and setting secrets from Cumulocity. Each is a follow-up change.

## Capabilities

### New Capabilities

- `connector-config-check`: the `tedge-dot check` command, its verdicts and its messages in both
  builds.
- `connector-config-plugin`: connector files as thin-edge.io configuration types: listing,
  snapshots, checked installs, new files and rollback.
- `device-management-ui`: the Device Management plugin's OT connectors page and device tab.

## Impact

- CLI and SDKs (C and Rust): the new subcommand, reusing the loader, the protocol modules'
  validation, the local-only settings lists and the path-reference check.
- Packaging: the plugin script in both packages (nfpm), next to the packaged configs.
- New `ui/` project (Web SDK 1024, Angular) with its own lockfile, lint and tests; CI and release
  workflows.
- Docs: a guide for managing connector files from Cumulocity, the install guide, the docs site
  map, and a release highlight.
- Users: no change on gateways that do not use configuration management. A gateway that registered
  a connector file by hand (the blueprint) lists it twice until that entry is removed; the docs
  explain the migration.
