# Spec Delta

## MODIFIED Requirements

### Requirement: Form view
The editor SHALL offer a form view and a TOML view of the same content. The form SHALL cover the
sections of the docs site's config builder: `[connector]` with `report`, `measurement` and
`grouping`, `[mqtt]`, `[connection]`, and each `[[device]]` with its settings, `protocol_address`
and points, with the builder's help text for each field. It SHALL show a schema error at the field
it concerns, add and remove devices and points, and switch the protocol. Keys the form does not
show SHALL be kept. A form edit SHALL change only the text it concerns, keeping the file's comments
and layout. When an edit cannot be written that way, the editor SHALL ask before it rewrites the
whole file in the config builder's layout. Files SHALL open in the form unless the form cannot
read them.

A device's `protocol_address` SHALL be shown after the device's name, poll interval and point
libraries, outside any collapsed section, and the device's heading SHALL show where it connects.
Only the address options of a protocol that most devices leave alone (for OPC UA: the password
file, the user certificate and key, and the plaintext-password, deprecated-security and
trust-any-certificate switches) SHALL be in the device's collapsed settings, with its type,
default mode and requests in flight. The collapsed settings SHALL open by themselves when one of
their fields has an error, or when an OPC UA device uses a deprecated policy without
`allow_deprecated_security`, and SHALL stay open when that is fixed.

For OPC UA, `security_policy` and `security_mode` in `[connection]` and in each device SHALL be
dropdowns: *Not set*, then the values both connector builds accept, with the deprecated policies
last and marked. The hint under each SHALL say what applies when it is not set. A value the
dropdown does not list SHALL stay selectable. Choosing the policy `None` SHALL set a `sign` or
`sign_and_encrypt` mode in the same table to `none`.

#### Scenario: Editing a field
- **WHEN** a user changes a Modbus device's `port` to `5020` in the form
- **THEN** the TOML view holds `port = 5020` in that device's `protocol_address`

#### Scenario: Error at its field
- **WHEN** a point's `datatype` is `"float33"`
- **THEN** the form shows the schema's message under that point's datatype field

#### Scenario: File with comments
- **WHEN** a user changes a point's `unit` in the form on `opcua-pump.toml`, which has comment lines
- **THEN** only that point's `unit` line changes in the TOML, and every comment line is still there

#### Scenario: New point in a commented file
- **WHEN** a user adds a point to the device of a commented file in the form
- **THEN** the TOML holds a new `[[device.point]]` table at the end of that device's section, and
  every comment line is still there

#### Scenario: Unknown key
- **WHEN** a point has a `meta` table the form has no field for, and the user changes its `unit`
- **THEN** the written file still holds the `meta` table

#### Scenario: Syntax error
- **WHEN** the content has an unclosed string on line 12
- **THEN** the form view shows no form and names line 12

#### Scenario: Address without opening anything
- **WHEN** a user opens an OPC UA file whose device has `endpoint = "opc.tcp://192.168.0.20:4840/"`
- **THEN** the endpoint field shows that value without any section being opened, and the device's
  heading shows `opc.tcp://192.168.0.20:4840/`

#### Scenario: Security dropdowns
- **WHEN** a user opens the security policy of an OPC UA device
- **THEN** the choices are *Not set*, `None`, `Basic256Sha256`, `Aes128_Sha256_RsaOaep`,
  `Aes256_Sha256_RsaPss`, then `Basic128Rsa15` and `Basic256` marked as deprecated, and the hint
  says which policy applies when it is not set

#### Scenario: None sets the mode
- **WHEN** a device has `security_policy = "Basic256Sha256"` and `security_mode = "sign"`, and the
  user chooses the policy `None`
- **THEN** the device holds `security_policy = "None"` and `security_mode = "none"`

#### Scenario: Deprecated policy opens the settings
- **WHEN** a user chooses `Basic256` for a device without `allow_deprecated_security`
- **THEN** the device's settings open with the deprecated-security switch, and stay open after the
  user turns it on

### Requirement: Validation before saving
Before saving, the editor SHALL validate the content with the docs site's config builder
validation and the contract schema. It SHALL mark syntax errors at their line and column, and list
schema errors with the key they concern. Content with errors SHALL not be saved. The UI and the
config builder SHALL give the same verdict for the same content.

For OPC UA, the validation SHALL also make the security checks both connector builds make: a
policy that is neither a known name nor a policy's full URI, a mode other than `none`, `sign` or
`sign_and_encrypt` (in any case, or `signandencrypt`), a deprecated policy without
`allow_deprecated_security`, the mode `none` with a secure policy, and `sign` or
`sign_and_encrypt` with the policy `None`. A device with its own policy and no mode SHALL be
checked with that policy's default mode. Each error SHALL be reported at the field that sets the
value, in the device or in `[connection]`.

#### Scenario: Syntax error
- **WHEN** a user saves content with an unclosed string on line 12
- **THEN** the editor marks line 12 and nothing is uploaded

#### Scenario: Schema error
- **WHEN** the fourth point of the first device declares `datatype = "float33"`
- **THEN** the editor lists the schema's message for that point's `datatype`, and nothing is uploaded

#### Scenario: Mode that does not fit the policy
- **WHEN** a device declares `security_policy = "Basic256Sha256"` and `security_mode = "none"`
- **THEN** the editor reports the error at that device's `security_mode`, and nothing is uploaded

#### Scenario: Mode from the connection
- **WHEN** `[connection]` declares `security_policy = "Basic256Sha256"` and `security_mode = "none"`,
  and a device sets neither
- **THEN** the editor reports the error at `[connection]`'s `security_mode`

#### Scenario: What the connectors accept
- **WHEN** a device declares the full URI of `Aes256_Sha256_RsaPss` as its policy, or
  `security_policy = "None"` with `security_mode = "None"`
- **THEN** the editor reports no security error
