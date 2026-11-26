### Connector editor: protocol address always shown, OPC UA security dropdowns

In the Device Management plugin's connector editor:

- A device's **Protocol address** is always shown, after its name, instead of inside the
  collapsed **Device settings**. The device's heading shows where it connects, such as its OPC UA
  endpoint. For OPC UA, the address options most devices leave alone (password file,
  certificates, and the trust, plaintext and deprecated-security switches) stay in Device
  settings.
- OPC UA **Security policy** and **Security mode** are dropdowns, in `[connection]` and in each
  device. The hint under each says what applies when it is not set. The deprecated policies come
  last and are marked, and choosing the policy `None` sets the mode next to it to `none`.
- The editor, and the docs site's config builder, now make the OPC UA connector's security checks:
  an unknown policy or mode, a deprecated policy without `allow_deprecated_security`, and a mode
  that does not fit the policy. Such a file can't be saved until it is fixed.

See [Connector files from Cumulocity](../doc/c8y-connector-files.md#form-and-toml).
