## 1. Shared checks and option lists

- [x] 1.1 `OPCUA_SECURITY_POLICIES` and `OPCUA_SECURITY_MODES` in `site/src/lib/form-options.ts`.
- [x] 1.2 `opcuaPolicy`, `opcuaMode`, `opcuaDefaultMode`, `opcuaDeviceSecurity` and the OPC UA checks in `validateConfig` (`site/src/lib/config-builder.ts`), following both connector builds.
- [x] 1.3 Site tests: accepted values (URI, mode in any case, alias), unknown names, deprecated with and without the switch, mode and policy mismatches on the device and in `[connection]`, a device's own policy not inheriting the connection's mode, other protocols untouched.
- [x] 1.4 Every OPC UA configuration in the repository passes the new checks.

## 2. Editor

- [x] 2.1 `ADDRESS_OPTIONS` and `addressSummary` in `ui/src/shared/form-model.ts`, with tests.
- [x] 2.2 `securityChoices` (options, notes, hints) and `linkSecurity` in `ui/src/shared/opcua-security.ts`, with tests, including the check errors through `validateConnectorFile`.
- [x] 2.3 `tdot-field` shows option notes; `tdot-shape` takes `only`, `skip`, `choices` and `primary`; the form state takes an `adjust` hook and keeps sections that opened by default open.
- [x] 2.4 Device layout: protocol address after name, poll interval and libraries; address options in Device settings; the address in the device heading; Device settings open on an error in them or a deprecated policy without the switch.
- [x] 2.5 Security dropdowns in `[connection]` and each device.

## 3. Docs and checks

- [x] 3.1 `doc/c8y-connector-files.md`: the device layout, the dropdowns and the OPC UA checks.
- [x] 3.2 Release highlight `changes/editor-protocol-address.md`.
- [x] 3.3 `npm test` and `npm run typecheck` in `ui/`, `npm test` in `site/`.
- [x] 3.4 In the dev server, on `tedge-dot-opcua-pump` without saving: the layout, the options, the hints, the mode error, `None` setting the mode, the deprecated policy opening Device settings, the switch clearing the error, and Modbus's layout.
