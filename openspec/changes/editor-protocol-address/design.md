## Context

The connector editor's form is generated from the config builder's protocol shapes
(`site/src/data/protocols.json`): `tdot-shape` renders a table's keys as fields, and a key with an
`enum` becomes a dropdown. The validation is the config builder's `validateConfig`, which
`ui/scripts/sync-site.mjs` copies into the plugin so the editor and the docs site give the same
verdict (requirement "Validation before saving").

Both OPC UA connector builds read the security settings the same way
(`impl/rust/crates/connector-opcua/src/config.rs`, `impl/c/connectors/opcua/connector_opcua.c`,
spec §3.3): a policy is its exact name or its full URI; a mode is read in any case, with
`signandencrypt` as an alias; a device's policy and mode win over `[connection]`'s, but a device
with its own policy and no mode gets that policy's default mode; a deprecated policy needs
`allow_deprecated_security` on the device or in `[connection]`.

## Goals / Non-Goals

**Goals:** the address visible without opening anything; dropdowns for policy and mode; the
connector's refusals caught before saving, at the field that causes them.

**Non-Goals:** dropdowns in the docs site's own form (it gets the checks, not the layout); new
`[connection]` fields such as `allow_deprecated_security` in the form; other protocols' fields.

## Decisions

- **D1: Checks in code, not an `enum` in the schema.** An `enum` on `security_policy` would refuse
  the full policy URI, and one on `security_mode` would refuse `None` or `Sign`, which the
  connectors accept. Instead `validateConfig` adds `opcuaSecurityErrors` for OPC UA files, written
  from the connectors' rules, and the form gets its options from lists in `form-options.ts`. A
  value the lists don't hold stays selectable, as with any choice field.
- **D2: The error goes where the value is set.** A device that takes its policy or mode from
  `[connection]` shows the problem at the `[connection]` field, because that is the line to change.
  An unknown name is reported wherever it is set.
- **D3: Deprecated policies listed and marked, not hidden.** Hiding them until
  `allow_deprecated_security` is on would hide a value a file already holds and hide the way to it.
  They come last, marked "deprecated", and choosing one without the switch shows the check error
  and opens Device settings, where the switch is.
- **D4: Only `None` changes the mode.** `None` allows only `none`, so choosing it sets a `sign` or
  `sign_and_encrypt` mode in the same table to `none` (`linkSecurity`, through a new `adjust` hook
  of the form state). Choosing a secure policy leaves a `none` mode as it is and shows the error,
  as #145 suggests, because the right mode (`sign` or `sign_and_encrypt`) is the user's choice.
- **D5: Which address keys are options.** `ADDRESS_OPTIONS` names them per protocol; only OPC UA
  has any. Every other key stays in the always-visible block, so other protocols look as before
  apart from the block's position. `tdot-shape` renders a subset of a table (`only`/`skip`), and only
  its primary part lists the keys the form has no field for.
- **D6: Sections stay open once they opened by themselves.** A section opened by an error used to
  close again when the error was fixed. With the deprecated-policy rule, turning on the switch
  would close the section under the cursor, so the form state now records a section that opened
  by default as open.

## Risks / Trade-offs

- A file whose policy the connector would never use (every device overrides it) still gets an
  error for an unknown name in `[connection]`. It is a typo either way.
- The docs site's config builder now rejects these combinations too, without the dropdowns.
