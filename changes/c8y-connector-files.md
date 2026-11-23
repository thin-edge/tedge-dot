### Connector files from Cumulocity

A new **Device Management plugin**, `tedge-dot-ui_<version>.zip` on the release, keeps connector
files and point libraries in Cumulocity's configuration repository:

- **Management > OT connectors** adds, edits, duplicates, deletes and sends connector files. The
  editor shows a file as a form, with the config builder's sections and help texts, or as TOML,
  and checks it against the contract schema with the same rules as the docs site's config
  builder. Form edits keep the file's comments.
- **Point libraries** live in the repository too, in a second list. A device picks its libraries
  from the ones that exist, the form shows the points it inherits, and sending a connector file to
  a gateway brings the libraries it references.
- An **OT connectors** tab on every thin-edge.io gateway shows its connector files like the
  Configuration tab, and adds a file the gateway does not have yet by registering it in
  thin-edge.io's list of managed files.
- Gateways need no change. A connector file is the configuration type `tedge-dot-<name>`, so files
  the OPC-UA solution blueprint registered show up as they are.

A file that tedge-dot refuses at load still counts as installed in Cumulocity. See
[Managing connector files from Cumulocity](../doc/c8y-connector-files.md).
