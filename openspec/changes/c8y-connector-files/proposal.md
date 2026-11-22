## Why

Connector files can reach a gateway from Cumulocity today, but only with manual steps on each
gateway:

- A file becomes a configuration type only when someone adds it to
  `/etc/tedge/plugins/tedge-configuration-plugin.toml` on the gateway, as the OPC-UA solution
  blueprint does. Until then Cumulocity cannot read it or send it.
- Nothing in Cumulocity shows or edits connector files. Users edit TOML locally, upload it to the
  configuration repository by hand and send it device by device, without any check before it
  reaches the gateway. Writing TOML by hand also needs the configuration reference open next to
  it.

## What Changes

- **Device Management plugin.** A new `ui/` project builds a Cumulocity Web SDK plugin for Device
  Management:
  - Management > OT connectors lists the configuration repository entries for connector files. It
    adds, edits, deletes and sends them to a gateway or a group. The editor checks each file
    against the contract schema, with the same validation as the docs site's config builder.
  - The editor shows a file as a form or as TOML. The form covers what the docs site's config
    builder covers (connector settings, reporting, grouping, MQTT, connection, devices and points,
    with the builder's help texts), so a file can be written without knowing its TOML keys. Form
    edits keep the file's comments and layout.
  - An "OT connectors" tab on thin-edge.io gateways shows the gateway's connector files and the
    repository's entries, using Device Management's own configuration components. It can add a
    connector file to the gateway and register a file that is already there.
- **Registration from Cumulocity.** To add a file, the UI appends an entry to the gateway's
  `tedge-configuration-plugin.toml` through configuration management, waits for thin-edge.io to
  offer the new type, then sends the file. Nothing has to be done on the gateway.
- **CI and releases.** CI builds and tests the UI. Releases attach the plugin zip.

Gateways need no change: thin-edge.io's built-in file plugin installs the files, and tedge-dot
0.0.13 or later applies them on its own.

Not in this change: a gateway-side plugin (discovering unregistered files, validation with
tedge-dot's own loader, rollback), per-gateway values, revisions and fleet views, point libraries
from Cumulocity, and secrets. Each is a follow-up change.

## Capabilities

### New Capabilities

- `device-management-ui`: the Device Management plugin's OT connectors page, its device tab, and
  how it registers connector files on gateways.

## Impact

- New `ui/` project (Web SDK, Angular) with its own lockfile, lint and tests, next to `site/`.
- The docs site's config builder: its validation, editing helpers, field options and help texts are
  shared with the UI instead of copied. The option lists move to a module of their own so the UI
  does not take the site's simulator code with them.
- CI and release workflows: a UI job and a release asset.
- Docs: a guide for managing connector files from Cumulocity, the install guide, the docs site
  map, and a release highlight.
- Gateways: none. Files registered by hand with the type `tedge-dot-<file name>`, as the blueprint
  does, show up in the UI as they are.
