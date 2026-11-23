# tedge-dot OT connectors

Edit the connector files of tedge-dot, thin-edge.io's OT connector service, in Cumulocity Device
Management, and install them on your gateways without logging in to them.

## What you get

- **Management > OT connectors**: your connector files, kept in the configuration repository. Add,
  upload, edit, and delete them, and send them to one gateway or to a group. Uploaded files are
  checked first; a file with problems opens in the editor.
- **A form and a TOML editor**: write a connector file in a form, with a short help text for every
  setting, or edit its TOML directly. Changes made in the form keep the file's comments. Before
  you save, the editor checks the file and shows each problem at the setting it concerns.
- **An OT connectors tab** on thin-edge.io devices: the device's connector files, with snapshots,
  saving to the repository, downloading, and sending, like the Configuration tab.
- **New files on a gateway**: when a gateway does not have a file yet, the plugin adds it to the
  gateway's list of managed files, shows you the entry for confirmation, and then sends the file.

## Requirements

- Cumulocity with Device Management 1025.
- Gateways with thin-edge.io 2.x and tedge-dot 0.0.13 or later.
- Permission to change the configuration repository (inventory admin) and to send operations
  (device control).

## Getting started

1. In **Administration > Ecosystem > Extensions**, add the plugin's zip file.
2. Add the **OT connectors** plugin to the Device Management application you use. Plugins go into
   a custom application, so copy the built-in Device Management first if you use that one.
3. Open **Management > OT connectors**, select **Add connector file**, pick the protocol to start
   from, fill in the form, and save.
4. Open a gateway, go to its **OT connectors** tab, select **Add connector file**, pick your file,
   and confirm.

## Good to know

- A connector file `/etc/tedge/plugins/ot/<name>.toml` appears in Cumulocity as the configuration
  type `tedge-dot-<name>`. Files that are already registered that way show up as they are.
- The check covers the file's syntax, settings, values, and protocol addresses. It cannot see what
  only the gateway knows, such as which point libraries are installed there. A file that tedge-dot
  refuses still shows as installed, so check the connector after you send a file.
- Sending to a group does not add files to gateways. The dialog lists the devices in the group
  that do not have the file yet.

The full guide is
[Managing connector files from Cumulocity](https://github.com/thin-edge/tedge-dot/blob/main/doc/c8y-connector-files.md).
