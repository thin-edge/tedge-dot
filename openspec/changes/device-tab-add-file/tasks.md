## 1. Helpers

- [x] 1.1 `managedFile(kind, name, protocol?)` in `naming.ts`: the file to register, or undefined for an invalid name or protocol; tests
- [x] 1.2 Editor context: parse and build the editor's `gateway`, `file` and `protocol` query parameters; tests
- [x] 1.3 Missing-file check over a gateway's configuration operations; tests

## 2. Device tab and dialog

- [x] 2.1 The dialog's three choices from the device tab; remove **Register a file on the gateway**
- [x] 2.2 *Already on the gateway* for connector files and point libraries, with the resulting path
- [x] 2.3 *New file* opens the editor for the gateway; hidden without the roles to add entries
- [x] 2.4 **Create connector file** / **Create point library** for a selected type without an entry
- [x] 2.5 The registered-but-missing hint, refreshed on selection, Reload and operation updates

## 3. Editor

- [x] 3.1 Fixed file name and protocol from the context
- [x] 3.2 With a gateway: Save opens the send dialog, then returns to the tab; Cancel returns to the tab
- [x] 3.3 **Edit in repository** from the tab passes the gateway

## 4. Service name

- [x] 4.1 `followFileName()` and the automatic-name rule in `service-name.ts`; tests
- [x] 4.2 The editor keeps a new connector file's service name in step with its file name
- [x] 4.3 Packaged example files' service names from the packaging at sync time; the editor warns about them and offers `tedge-dot-<file name>`

## 5. Docs and checks

- [x] 5.1 Update the OT connectors tab section of `doc/c8y-connector-files.md`
- [x] 5.2 Release highlight in `changes/device-tab-add-file.md`
- [x] 5.3 Unit tests, type check and build pass
- [x] 5.4 Check on a tenant: each choice, creating from a type, the hint, and the failure text
