# tedge-dot OT connectors for Device Management

A Cumulocity plugin for Device Management that keeps tedge-dot connector files in the
configuration repository, checks and edits them in one place, and installs them on thin-edge.io
gateways. Gateways need no change: thin-edge.io's built-in file plugin installs the files, and
tedge-dot 0.0.13 or later applies them on its own.

- **Management > OT connectors** lists the repository's connector files. Add, edit, delete, and
  send a file to a gateway or to a group. The editor checks each file against the tedge-dot
  contract schema before it saves.
- **OT connectors** tab on thin-edge.io gateways: the gateway's connector files with snapshots,
  saving to the repository, downloading and sending, like the Configuration tab. *Add connector
  file* installs a file the gateway does not have yet, and *Register a file on the gateway* makes
  an existing file, such as a packaged `modbus.toml`, available to Cumulocity.

The guide is [doc/c8y-connector-files.md](https://github.com/thin-edge/tedge-dot/blob/main/doc/c8y-connector-files.md).

## Development

The project needs Node.js 22 or later.

```sh
npm ci
npm test            # unit tests (Jest, no browser)
npm run typecheck
npm run build       # dist/tedge-dot-ui.zip
npm start -- --url https://<tenant>.cumulocity.com   # Device Management with this plugin loaded
```

`npm run sync` (run before every build and test) copies the docs site's connector-file
validation (`site/src/lib/config-builder.ts`, `site/src/data/protocols.json`) and the contract
schemas into `src/generated/`, so the plugin and the config builder share one set of rules.

The logic lives in plain TypeScript under `src/shared/` and is unit-tested without Angular. The
components under `src/connectors/` and `src/device-tab/` use the Web SDK (1025).
