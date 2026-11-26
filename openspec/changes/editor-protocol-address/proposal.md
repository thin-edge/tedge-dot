## Why

In the connector editor, the fields that say where a device is (for OPC UA: the endpoint,
security policy and mode, user and certificates) sit in a device's **Device settings**, together
with its type, default mode and requests in flight. That section starts collapsed and only opens
by itself when a device field has an error, so the address looks as if it can't be edited
(thin-edge/tedge-dot#144).

OPC UA **Security policy** and **Security mode** are free-text fields, although each takes a fixed
set of values, and the two are spelled differently (`None`, but `none`). A typo, a deprecated
policy without `allow_deprecated_security`, or a mode that does not fit the policy only shows up
when the connector refuses the device on the gateway (thin-edge/tedge-dot#145).

## What Changes

- **Protocol address first.** Each device shows its **Protocol address** after its name, poll
  interval and point libraries, always visible. The device's heading shows where it connects (the
  OPC UA endpoint, the Modbus host and port, and so on).
- **Address options stay collapsed.** For OPC UA, the address options most devices leave alone
  (password file, user certificate and key, and the plaintext-password, deprecated-security and
  trust-any-certificate switches) move into **Device settings** with the type, default mode and
  requests in flight. Device settings open by themselves when one of their fields has an error, or
  when the device uses a deprecated policy without allowing it, and stay open once fixed.
- **Security dropdowns.** OPC UA **Security policy** and **Security mode** are dropdowns in
  `[connection]` and in each device: *Not set* first, then the values the connectors know, the
  deprecated policies last and marked. The hint under each says what applies when it is not set.
  Choosing the policy `None` sets a `sign` or `sign_and_encrypt` mode next to it to `none`.
- **Security checks.** The shared config builder validation (used by the editor and the docs
  site's config builder) makes the OPC UA connector's security checks: an unknown policy or mode,
  a deprecated policy without `allow_deprecated_security`, the mode `none` with a secure policy,
  and `sign` or `sign_and_encrypt` with `None`. The error shows at the field that sets the value.

## Impact

- `ui/`: the connector form, `ui/src/shared/form-model.ts`, and a new
  `ui/src/shared/opcua-security.ts`.
- `site/src/lib/config-builder.ts` and `site/src/lib/form-options.ts`: the checks and the option
  lists, shared with the docs site's config builder, whose verdict changes the same way. The
  site's own form is unchanged.
- Docs: the form and checks sections of `doc/c8y-connector-files.md`; a release highlight.
- No change on gateways. Every OPC UA configuration in the repository passes the new checks.
