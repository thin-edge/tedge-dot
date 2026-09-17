# Spec Delta

## Purpose

Defines how the OPC UA connectors (Rust and C) establish secured sessions to OPC UA servers.
This covers security settings, the connector's application certificate, server certificate
trust, user identity, endpoint selection, handling of secrets and failure reporting. Both
implementations behave the same way.

## ADDED Requirements

### Requirement: Security policy and mode configuration
The connector SHALL accept `security_policy` and `security_mode` in `[connection]` as defaults
and in `device.protocol_address` as per-device overrides. The per-device value wins.

Supported policies:
- `None`, `Basic256Sha256`, `Aes128_Sha256_RsaOaep` and `Aes256_Sha256_RsaPss`.
- `Basic128Rsa15` and `Basic256` are deprecated. They SHALL be refused unless
  `allow_deprecated_security = true` is set in the same scope or in `[connection]`.

Supported modes: `none`, `sign` and `sign_and_encrypt`.

A policy of `None` SHALL require mode `none`, and any other policy SHALL require `sign` or
`sign_and_encrypt`. When both settings are omitted, the effective values SHALL be `None` and
`none`.

An unknown value, a deprecated policy that is not allowed, or an inconsistent pair SHALL be a
configuration validation error. The error SHALL name the device and the offending field.

#### Scenario: Per-device override
- **WHEN** `[connection]` sets `security_policy = "Basic256Sha256"` and a device sets `security_policy = "None"` with `security_mode = "none"`
- **THEN** that device connects without message security and every other device uses `Basic256Sha256`

#### Scenario: Inconsistent pair is rejected
- **WHEN** a device sets `security_policy = "Basic256Sha256"` and `security_mode = "none"`
- **THEN** configuration validation fails with an error naming the device and `security_mode`

#### Scenario: Deprecated policy requires opt-in
- **WHEN** a device sets `security_policy = "Basic256"` without `allow_deprecated_security = true`
- **THEN** configuration validation fails with an error stating that the policy is deprecated and how to allow it

#### Scenario: Defaults stay unsecured
- **WHEN** neither `[connection]` nor the device sets a policy or mode
- **THEN** the device connects with policy `None` and mode `none`, and no application certificate is required

### Requirement: Application instance certificate
When at least one device uses a policy other than `None`, the connector SHALL use an
application instance certificate and private key. It SHALL obtain them in this order:
1. The files named by `[connection] certificate` and `private_key`, when both are set.
2. The PKI directory's `own/` entries (see `opcua-pki-management`).
3. A newly generated certificate, when neither exists and `create_certificate` is not `false`
   (the default is `true`).

A generated certificate SHALL meet all of the following:
- It is self-signed, with an RSA key of at least 2048 bits.
- Its URI SAN equals `application_uri`, and its DNS SAN holds the host name.
- It is valid for no more than 5 years.
- It is written to `own/` with the private key readable only by the owner (mode 0600).
- It is reused on every later start.

The following SHALL be configuration errors for the affected devices:
- The configured or stored certificate has no URI SAN equal to `application_uri`.
- The key does not match the certificate.
- No certificate can be obtained.

The connector SHALL log a warning at startup and once a day when its certificate expires
within 30 days. When the certificate has expired, secured devices SHALL be `disconnected` with
a reason stating that.

#### Scenario: Certificate generated on first secured start
- **WHEN** a device uses `Basic256Sha256`, no certificate is configured, and `own/` is empty
- **THEN** the connector creates a self-signed certificate whose URI SAN equals `application_uri`, stores it and its key (mode 0600) under `own/`, and connects with it

#### Scenario: Certificate reused across restarts
- **WHEN** the connector restarts after generating its certificate
- **THEN** it presents the same certificate (same thumbprint) and does not generate a new one

#### Scenario: Application URI mismatch
- **WHEN** the configured certificate's URI SAN is `urn:other` and `application_uri` is `urn:tedge-dot`
- **THEN** the secured devices are not connected and the error names both URIs

#### Scenario: Unsecured-only configuration needs no certificate
- **WHEN** every device uses policy `None`
- **THEN** no certificate is generated and no PKI directory is created

### Requirement: Server certificate validation
For every secured session, the connector SHALL validate the server certificate before it
activates the session. The certificate is trusted only if one of these holds:
- **Pinned**: the identical certificate is in `trusted/certs/`.
- **CA-issued**: it chains to a CA certificate in `trusted/certs/`, with any intermediates
  taken from `issuers/certs/` or `trusted/certs/`.

For a CA-issued certificate, every CA in the chain SHALL have a CRL in the matching `crl/`
directory. The certificate SHALL be rejected when any certificate in the chain is revoked, or
when a CA in the chain has no CRL (revocation unknown).

In both cases the connector SHALL also reject the certificate when any of these checks fails:
- It is outside its validity period.
- Its SAN has no DNS name or IP address matching the configured endpoint host.
- Its URI SAN does not match the server's advertised `applicationUri`.
- Its key length is invalid for the effective policy.

`rejected/certs/` SHALL be a list for review, not a deny list: a certificate that is pinned or
chains to a trusted CA SHALL be trusted even when a copy is also in `rejected/certs/`. A single
certificate issued by a trusted CA is distrusted by revoking it.

#### Scenario: Pinned self-signed server certificate
- **WHEN** the server's self-signed certificate has been copied to `trusted/certs/`
- **THEN** the session is established

#### Scenario: CA trusted after the server certificate was quarantined
- **WHEN** a CA-issued server certificate was copied to `rejected/certs/` and the operator then adds the CA and its CRL to `trusted/`
- **THEN** the next session is established, without the operator having to remove the copy from `rejected/certs/`

#### Scenario: CA-issued server certificate
- **WHEN** the server certificate is issued by a CA whose certificate is in `trusted/certs/` and whose CRL is in `trusted/crl/`, and the server certificate is not revoked
- **THEN** the session is established without the server certificate itself being in `trusted/`

#### Scenario: Revoked server certificate
- **WHEN** the CA's CRL lists the server certificate's serial number
- **THEN** the session is refused and the link-status reason states that the certificate is revoked

#### Scenario: Missing CRL
- **WHEN** the server certificate chains to a trusted CA but no CRL for that CA is present
- **THEN** the session is refused and the reason states that revocation status is unknown

#### Scenario: Host name mismatch
- **WHEN** the endpoint is `opc.tcp://10.0.0.5:4840/` and the trusted server certificate names only `plc.example.com`
- **THEN** the session is refused and the reason states the host name mismatch

#### Scenario: Expired server certificate
- **WHEN** the trusted server certificate's `notAfter` is in the past
- **THEN** the session is refused and the reason states that the certificate has expired

### Requirement: Untrusted server certificates are quarantined, not fatal
When a server certificate is untrusted, the connector SHALL do all of the following:
- Store a copy in `rejected/certs/`, named so that it can be identified by thumbprint.
- Keep that device `disconnected` with a reason that contains the text `untrusted`, the
  certificate's SHA-1 thumbprint and its subject.
- Keep retrying with the normal reconnect backoff.
- Keep serving every other device.

After the operator trusts the certificate, the next reconnect attempt SHALL succeed without a
restart or reload.

#### Scenario: First contact with an unknown server
- **WHEN** a device connects to a server whose certificate is neither pinned nor CA-issued by a trusted CA
- **THEN** the certificate appears in `rejected/certs/`, the device's link status is `disconnected` with a reason containing `untrusted` and the thumbprint, and other devices keep publishing samples

#### Scenario: Trust takes effect without restart
- **WHEN** the operator moves that certificate into `trusted/certs/` while the connector is running
- **THEN** the device becomes `connected` on a later reconnect attempt, and the connector process was not restarted

### Requirement: Explicit opt-out of server certificate validation
The connector SHALL support `trust_any_server_certificate = true` in `[connection]` or in a
device's `protocol_address`. With it set, any server certificate is accepted for the affected
devices, and the message is still signed or encrypted as configured. The option SHALL default
to `false`.

While the option is in effect, the connector SHALL log a warning for each affected device on
every connect. The link-status `info` SHALL include `"server_certificate": "not_verified"`.

#### Scenario: Opt-out accepts an unknown certificate
- **WHEN** a device sets `trust_any_server_certificate = true` and the server certificate is not trusted
- **THEN** the session is established, a warning is logged, and link-status `info.server_certificate` is `"not_verified"`

#### Scenario: Default does not trust unknown certificates
- **WHEN** an existing configuration that uses `Basic256Sha256` does not set `trust_any_server_certificate`
- **THEN** an untrusted server certificate is rejected

### Requirement: Endpoint selection keeps the configured address
For a secured device, the connector SHALL query the server's endpoints and select the one that
matches the effective policy, the effective mode and the required user token type. It SHALL
dial the host and port of the configured `endpoint`, even when the selected endpoint advertises
a different host name or port. When no advertised endpoint matches, the device SHALL be
`disconnected` with a reason that lists the policy and mode pairs the server offers.

#### Scenario: Server advertises an unreachable host name
- **WHEN** the device is configured with `opc.tcp://192.168.1.20:4840/` and the server advertises `opc.tcp://plc-internal:4840/`
- **THEN** the secured session is established over `192.168.1.20:4840`

#### Scenario: No matching endpoint
- **WHEN** the device requests `Aes256_Sha256_RsaPss`/`sign_and_encrypt` and the server offers only `None`/`none` and `Basic256Sha256`/`sign`
- **THEN** the device is `disconnected` and the reason lists `None/none` and `Basic256Sha256/sign`

### Requirement: User identity tokens
Each device SHALL authenticate with exactly one of these identities:
- **Anonymous**: the default.
- **Username**: `user` together with at most one of `password` or `password_file`.
- **X.509 user certificate**: `user_certificate` together with `user_private_key`.

Setting both `password` and `password_file`, or mixing username and certificate fields, SHALL
be a validation error. A `password_file` SHALL be read when the configuration is loaded, with
the first line used and the trailing newline stripped. An unreadable file SHALL be a validation
error that names the path but not the contents.

The connector SHALL sign or encrypt the token as the matching user token policy requires. When
the server offers no user token policy for the configured identity type, the device SHALL be
`disconnected` with a reason saying which identity type is unsupported.

#### Scenario: Username with password file
- **WHEN** a device sets `user = "operator"` and `password_file = "/etc/tedge/plugins/ot/secrets/plc1"` and the server accepts those credentials
- **THEN** the session is activated as `operator`

#### Scenario: X.509 user identity
- **WHEN** a device sets `user_certificate` and `user_private_key` and the server trusts that user certificate
- **THEN** the session is activated with the X.509 identity

#### Scenario: Rejected credentials
- **WHEN** the server rejects the username or password
- **THEN** the device is `disconnected` with a reason stating that the identity was rejected, and the reason does not contain the password

#### Scenario: Ambiguous identity
- **WHEN** a device sets both `password` and `password_file`
- **THEN** configuration validation fails naming the device and both fields

### Requirement: No plaintext passwords by default
The connector SHALL refuse to send a password that would be unprotected, which is the case when
either of these holds:
- The channel has no message security (mode `none`), whatever the username token policy names.
  Table 193 of OPC UA Part 4 allows token encryption there, but nothing on such a channel
  authenticates the server certificate the password would be encrypted to.
- The channel is signed only (mode `sign`) and the username token policy names `None`.

On `sign_and_encrypt`, or on `sign` with a token policy that is unnamed (the channel's policy)
or names a real policy, the server certificate was validated and the password is protected.

When the connector refuses, the device SHALL be `disconnected` with a reason explaining this.
The refusal SHALL NOT apply when `allow_plaintext_password = true` is set for the device or in
`[connection]`. In that case the connector SHALL log a warning on each connect.

#### Scenario: Password over an unsecured channel is refused
- **WHEN** a device uses policy `None` and `user`/`password`, and the server's username token policy has no security policy
- **THEN** no session activation is attempted and the reason states that the password would be sent in plaintext

#### Scenario: Token-level encryption on an unsecured channel is not enough
- **WHEN** a device uses policy `None` with `user`/`password`, and the server's username token policy specifies `Basic256Sha256`
- **THEN** no session activation is attempted unless `allow_plaintext_password = true`, because no server certificate was authenticated

### Requirement: Secrets are never disclosed
Passwords, password file contents and private key material SHALL NOT appear in any of these
places:
- log output, at any level
- link-status messages
- service health messages
- command responses
- `describe` output
- configuration validation errors

#### Scenario: Validation error with a password present
- **WHEN** a device configuration with `password = "s3cret"` fails validation for any reason
- **THEN** the error output does not contain `s3cret`

#### Scenario: Link status of a secured device
- **WHEN** a device with username and password connects
- **THEN** its retained link-status message contains neither the password nor any private key material

### Requirement: Management commands cannot set local-only settings
A management command (`set-config`, `define-device`) SHALL be refused when it adds or changes
a setting that names a file on the gateway or relaxes security: `pki_dir`, `certificate`,
`private_key`, `create_certificate`, `password_file`, `user_certificate`, `user_private_key`,
`trust_any_server_certificate`, `allow_plaintext_password` and `allow_deprecated_security`, in
`[connection]` or a device's `protocol_address`, or removes one from a device that remains.
Values the configuration already has SHALL stay accepted, and removing a whole device SHALL be
allowed.

#### Scenario: define-device with a password file
- **WHEN** a `define-device` command gives a new device `password_file = "/etc/shadow"`
- **THEN** the command fails with a reason naming `password_file` but not the path, and the configuration file is unchanged

#### Scenario: Unrelated command on a configuration that uses a password file
- **WHEN** the configuration has a device with `password_file` and a `set-config` command changes that device's `poll_interval`
- **THEN** the command succeeds

### Requirement: Security state in link status
For every device, link-status `info` SHALL include the effective `security_policy` and
`security_mode`. For secured devices it SHALL also include:
- `server_certificate`: `"trusted"` or `"not_verified"`
- the server certificate's thumbprint, once known

A `disconnected` reason caused by security SHALL start with one of these category prefixes:
- `certificate untrusted:`
- `certificate invalid:`
- `certificate revoked:`
- `no matching endpoint:`
- `identity rejected:`
- `identity unsupported:`
- `plaintext password refused:`
- `application certificate:`

#### Scenario: Connected secured device
- **WHEN** a device is connected with `Basic256Sha256`/`sign_and_encrypt` to a trusted server
- **THEN** its link-status `info` contains `security_policy = "Basic256Sha256"`, `security_mode = "sign_and_encrypt"`, `server_certificate = "trusted"` and the server certificate thumbprint

#### Scenario: Categorised failure reason
- **WHEN** a device fails because its server certificate is untrusted
- **THEN** its link-status reason starts with `certificate untrusted:`

### Requirement: Implementation parity
The Rust and C connectors SHALL both implement every requirement in this capability. The C
build SHALL no longer report `opcua-security` as a parity gap. Given the same configuration,
PKI directory and server, both implementations SHALL produce the same connection outcome and
the same link-status reason category.

#### Scenario: Same suite, both implementations
- **WHEN** the secured OPC UA e2e and conformance suites run with `IMPL=rust` and with `IMPL=c`
- **THEN** both runs pass without implementation-specific skips for security
