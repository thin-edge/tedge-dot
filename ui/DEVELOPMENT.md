# Developing the OT connectors plugin

The Device Management plugin in this folder is a Cumulocity Web SDK 1025 project (Angular 22).
`README.md` is what users see on the extension's page in Cumulocity; this file is for working on
the code.

The project needs Node.js 22 or later.

```sh
npm ci
npm test            # unit tests (Jest, no browser)
npm run typecheck
npm run build       # dist/tedge-dot-ui.zip
npm start -- --url https://<tenant>.cumulocity.com   # Device Management with this plugin loaded
```

`npm run sync` (run before every build and test) copies the docs site's connector-file model into
`src/generated/`: the config builder (`site/src/lib/config-builder.ts`) with its protocol shapes
and the contract schemas, the forms' option lists (`site/src/lib/form-options.ts`), and the field
help (`site/src/lib/help.ts`). The plugin and the docs site's config builder share one set of rules.

The logic lives in plain TypeScript under `src/shared/` and is unit-tested without Angular. The
components under `src/connectors/` and `src/device-tab/` use the Web SDK.

Form edits are written into a file's text with `@decimalturn/toml-patch`, so comments stay
(`src/shared/toml-write.ts`). Every result is parsed again and compared with the form's values;
when that fails, the editor asks before it rewrites the file.

Device Management 1025 runs without zone.js change detection: keep component state in signals.

The release workflow sets the version from the release; `package.json` keeps the version
release-please writes.
