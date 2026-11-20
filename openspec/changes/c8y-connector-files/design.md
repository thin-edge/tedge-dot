## Context

thin-edge.io (2.1 and later) runs configuration plugins from `/usr/share/tedge/config-plugins`
through sudo. A plugin answers `list`, `get <type>`, `prepare <type> <file> --work-dir <dir>`,
`set <type> <file> --work-dir <dir>`, `verify <type> --work-dir <dir>` and
`rollback <type> --work-dir <dir>`. Its types are published as `<type>::<plugin>`, and the suffix is
stripped before the plugin sees the type. The agent re-runs `list` when a plugin is installed, when
software is installed, and on the signal `te/device/main/service/tedge-agent/signal/sync_config`.
An update runs download, `prepare`, `set`, `verify`. A failed `prepare` fails the operation
without rollback. A failed `set` or `verify` runs `rollback`, then fails the operation. The c8y
mapper turns the types into `c8y_SupportedConfigurations` and the operations
`c8y_UploadConfigFile` and `c8y_DownloadConfigFile` into these commands.

tedge-dot runs every file in `/etc/tedge/plugins/ot` in one service as the user `tedge`. It applies
a changed file within the watch interval (default 2 s) and keeps the previous configuration when a
file cannot be used. Management commands over MQTT are checked against two rules that whole files
skip today: no added or changed local-only settings (`LOCAL_ONLY_SETTINGS` per protocol: files whose
contents would be sent to a server, and settings that relax server authentication), and no new
path references in `points_from`.

In Cumulocity, a configuration repository entry is a managed object of type
`c8y_ConfigurationDump` with `name`, `description`, `configurationType`, `deviceType` and `url` (a
binary). Device Management offers an entry to devices whose supported configurations contain its
type and whose type matches its device type, when one is set.

## Goals / Non-Goals

**Goals:**
- Connector files are configuration types on every gateway with the package, with no manual
  registration.
- Cumulocity can add a connector file to a gateway that does not have it yet.
- A file from Cumulocity passes the same validation as a local file, and the same remote-change
  rules as a management command, before it replaces anything.
- One place in Device Management to keep, edit and send connector files, and a device tab that
  follows the Configuration tab.

**Non-Goals:**
- Per-gateway values, a form-based editor, revisions, a fleet view, point libraries and secrets
  from Cumulocity. These are follow-up changes.
- Changing the contract, the configuration model or the management commands.

## Decisions

### D1. A plugin instead of entries in the file plugin's list

Entries in `tedge-configuration-plugin.toml` are static, cannot add a file, and write without
checking. A plugin lists the directory as it is, can create a file, can refuse a file before it
installs it, and can restore the previous file. It needs no change in thin-edge.io.

### D2. The type is the file name

A file `/etc/tedge/plugins/ot/<name>.toml` is the type `<name>::tedge-dot`. File names are already
unique in the directory, and editing a file's content never changes its type. Naming types after
`service_name` was rejected: two files can share one by mistake, and renaming the service would
turn the same file into a new type. Names match `[A-Za-z0-9][A-Za-z0-9._-]*`, so no name can leave
the directory.

### D3. Validation in `prepare`, with the service's own code

`prepare` runs `tedge-dot check <file> --as <dir>/<name>.toml --remote`. A failure there ends the
operation before any file changes, and its message becomes the operation's failure reason. The
check is a subcommand of each build, so it uses the same loader, point library resolution and
protocol-module configuration checks as the service, without connecting. A file that passes is a
file the service accepts. The plugin runs the check as the user `tedge` (`runuser`), not as root,
because the file comes from the cloud.

### D4. Whole files follow the management-command rules

`--remote` compares the new file with the installed one and refuses an added or changed
local-only setting, a new path reference in `points_from`, and a changed `point_library_path`. The
last one closes the same hole as path references: a cloud file could otherwise point library
lookups at any directory. For a new file, every such setting counts as added.

Consequence: Cumulocity can update a file that already names a password file, as long as the setting
stays the same, but it cannot introduce one. A gateway gets its first file with a password file
locally. Setting secrets from Cumulocity is a follow-up change with its own security review.

### D5. Atomic install, then the configuration watch

`set` writes the file next to the target and renames it into place, keeping an existing file's owner
and mode (new files: `tedge:tedge`, `0644`). The service picks it up through its watch, so the
plugin never restarts or signals the service. For a new file, `set` publishes `sync_config` so the
new type reaches Cumulocity.

### D6. `verify` compares bytes, `rollback` restores

`verify` succeeds when the installed file equals the checked copy in the work directory. It does
not wait for the service: the check already used the service's loader, and problems at runtime,
such as an unreachable device, belong in the link status rather than in a failed operation.
`rollback` restores the copy kept by `prepare`, or removes a file the update created and publishes
`sync_config`.

### D7. A shell plugin shared by both packages

The plugin only moves files and calls `tedge-dot check`, so it is one POSIX shell script in
`packaging/`, installed by both packages at `/usr/share/tedge/config-plugins/tedge-dot`, which
thin-edge.io's sudoers rule already covers.

### D8. A Device Management plugin in `ui/`

The UI is a Web SDK 1024 plugin with its own `package.json` and lockfile in `ui/`, built and tested
in CI and attached to releases as a zip. Tenants install it as an extension and add it to their
Device Management application. It adds:
- a Management navigator node and page "OT connectors" that lists repository entries whose type
  ends with `::tedge-dot`, with add, edit, delete and send;
- a device tab "OT connectors", shown when the device's `c8y_SupportedConfigurations` contains a
  `::tedge-dot` type.

### D9. The browser checks syntax only

The editor checks TOML syntax before saving and shows the line and column of an error. It does not
reimplement tedge-dot's rules. The gateway's check is the single authority, and its message
reaches the user as the operation's failure reason.

### D10. Sending uses the platform's operations

One gateway gets a `c8y_DownloadConfigFile` operation with the entry's type and binary URL. A group
gets a bulk operation with the same operation. Adding a file to a gateway uses the same operation
with a type the gateway does not list yet: the plugin creates it (D5).

## Risks / Trade-offs

- [The agent may refuse a type its plugin does not list] → The first task tries it on a gateway.
  If the agent refuses it, the plugin lists a fixed type `new-connector` whose `set` names the file
  after the content's `service_name`.
- [A file registered by hand in `tedge-configuration-plugin.toml` shows up twice] → The docs
  explain how to remove the manual entry. The plugin does not edit that file.
- [The plugin runs as root] → It accepts only names that match D2, never runs content, and runs
  the check as `tedge`.
- [The PR is large: CLI in two builds, a plugin, a UI] → `ui/` is self-contained, and the PR is
  split into commits per part.
- [Node tooling in a C and Rust repository] → It stays inside `ui/`, like `site/`, with its own CI
  job.

## Open Questions

- The oldest thin-edge.io release with configuration plugins. The docs state no version; 2.1.0 is
  tested.
- How the UI zip is versioned: with the packages through release-please, or separately.
- Management commands do not refuse a changed `point_library_path` today. Should `set-config`
  refuse it like `--remote` does (D4)? That would change the management commands and is left out
  of this change.
