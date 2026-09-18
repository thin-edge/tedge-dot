# Spec Delta

## ADDED Requirements

### Requirement: A server rejecting the application certificate is reported distinctly
When the connector's own validation of the server certificate has passed — or was opted out of
with `trust_any_server_certificate` — and the handshake or session activation still fails
because a certificate is untrusted, the failure is the server's judgement of the connector's
application certificate, not the connector's judgement of the server's.

The connector SHALL report that case with the reason category `application certificate
rejected:`. The reason SHALL name the connector's own certificate by its SHA-1 thumbprint, and
SHALL state that the server administrator has to trust it. The subject is not required: the
certificate is the connector's own, so the thumbprint and `tedge-dot pki export` identify it. The reason SHALL NOT
state that the server's certificate is untrusted.

The device SHALL stay `disconnected` and keep retrying with the normal reconnect backoff, and
SHALL connect on a later attempt once the server trusts the certificate, without the connector
being restarted.

#### Scenario: Server does not trust the connector
- **WHEN** the server certificate is trusted by the connector but the server refuses the
  session because it does not trust the connector's application certificate
- **THEN** the device's reason starts with `application certificate rejected:`, names the
  connector's own thumbprint, and does not say that the server's certificate is untrusted

#### Scenario: Connects after the server is given the certificate
- **WHEN** the operator hands `tedge-dot pki export` output to the server administrator and the
  server trusts it
- **THEN** the device becomes `connected` on a later reconnect attempt, with no restart

#### Scenario: Our own distrust is still reported as untrusted
- **WHEN** the server certificate is neither pinned nor CA-issued by a trusted CA
- **THEN** the reason still starts with `certificate untrusted:` and not with `application
  certificate rejected:`

### Requirement: Secure channel tokens are renewed
For a secured session, the connector SHALL renew the secure channel's security token before it
expires, and the session SHALL survive the renewal: polling and subscriptions continue, no
sample is lost to the renewal, and the device's link status stays `connected` throughout.

The connector SHALL honour the token lifetime the server grants, which may be shorter than the
one requested. A failure to renew SHALL be handled as a lost connection: the device becomes
`disconnected` with a reason and reconnects with the normal backoff.

#### Scenario: Session outlives several token lifetimes
- **WHEN** a device is connected to a server that grants a 30-second token lifetime and runs
  for several minutes
- **THEN** the device stays `connected` throughout, samples keep arriving at the configured
  interval, and no reconnect is logged

#### Scenario: Subscriptions survive renewal
- **WHEN** a subscribed point's value changes after the security token has been renewed
- **THEN** the sample is still delivered on the push path

## MODIFIED Requirements

### Requirement: Explicit opt-out of server certificate validation
The connector SHALL support `trust_any_server_certificate = true` in `[connection]` or in a
device's `protocol_address`. With it set, the connector SHALL NOT judge *who* the server is:
the certificate is accepted whether or not it is pinned, chains to a trusted CA, is within its
validity period, names the endpoint host or matches the server's `applicationUri`. The message
is still signed or encrypted as configured. The option SHALL default to `false`.

The option SHALL NOT waive the requirements the security policy itself places on the
certificate. In particular the key length SHALL still be checked against the effective policy,
and a key outside its range SHALL be refused with `certificate invalid:` even when the option is
set. Trusting a certificate cannot change its key, and an implementation whose crypto stack
enforces the range below the connector would refuse the session regardless.

While the option is in effect, the connector SHALL log a warning for each affected device on
every connect. The link-status `info` SHALL include `"server_certificate": "not_verified"`.

#### Scenario: Opt-out accepts an unknown certificate
- **WHEN** a device sets `trust_any_server_certificate = true` and the server certificate is not trusted
- **THEN** the session is established, a warning is logged, and link-status `info.server_certificate` is `"not_verified"`

#### Scenario: Default does not trust unknown certificates
- **WHEN** an existing configuration that uses `Basic256Sha256` does not set `trust_any_server_certificate`
- **THEN** an untrusted server certificate is rejected

#### Scenario: Opt-out does not waive the policy's key length
- **WHEN** a device sets `trust_any_server_certificate = true` with policy `Basic256Sha256` and
  the server presents a 1024-bit certificate
- **THEN** the device is `disconnected`, the reason starts with `certificate invalid:` and names
  the key size and the policy


### Requirement: Endpoint selection keeps the configured address
For a secured device, the connector SHALL query the server's endpoints and select the one that
matches the effective policy, the effective mode and the required user token type. It SHALL
dial the host and port of the configured `endpoint`, even when the selected endpoint advertises
a different host name or port. It SHALL likewise keep the resource path of the configured
`endpoint`, whether that path is empty or not, rather than the path of the advertised endpoint
URL.

When no advertised endpoint matches, the device SHALL be `disconnected` with a reason that
lists the policy and mode pairs the server offers. That list SHALL include pairs whose policy
the connector does not implement, named as the server advertises them, so that the reason
explains why nothing matched instead of appearing to list nothing.

#### Scenario: Server advertises an unreachable host name
- **WHEN** the device is configured with `opc.tcp://192.168.1.20:4840/` and the server advertises `opc.tcp://plc-internal:4840/`
- **THEN** the secured session is established over `192.168.1.20:4840`

#### Scenario: Configured resource path is kept
- **WHEN** the device is configured with `opc.tcp://plc:4840/UA/Server` and the matching
  advertised endpoint names a different host
- **THEN** the session is established to `plc:4840` with the resource path `/UA/Server`

#### Scenario: No matching endpoint
- **WHEN** the device requests `Aes256_Sha256_RsaPss`/`sign_and_encrypt` and the server offers only `None`/`none` and `Basic256Sha256`/`sign`
- **THEN** the device is `disconnected` and the reason lists `None/none` and `Basic256Sha256/sign`

#### Scenario: Policies the connector does not implement are still listed
- **WHEN** the server offers only policies the connector does not implement, such as elliptic
  curve policies
- **THEN** the device is `disconnected` and the reason lists those policies as the server named
  them

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
- `application certificate rejected:`

#### Scenario: Connected secured device
- **WHEN** a device is connected with `Basic256Sha256`/`sign_and_encrypt` to a trusted server
- **THEN** its link-status `info` contains `security_policy = "Basic256Sha256"`, `security_mode = "sign_and_encrypt"`, `server_certificate = "trusted"` and the server certificate thumbprint

#### Scenario: Categorised failure reason
- **WHEN** a device fails because its server certificate is untrusted
- **THEN** its link-status reason starts with `certificate untrusted:`

#### Scenario: Server-side rejection is its own category
- **WHEN** a device fails because the server does not trust the connector's application
  certificate
- **THEN** its link-status reason starts with `application certificate rejected:`

### Requirement: Implementation parity
The Rust and C connectors SHALL both implement every requirement in this capability. The C
build SHALL no longer report `opcua-security` as a parity gap. Given the same configuration,
PKI directory and server, both implementations SHALL produce the same connection outcome and
the same link-status reason category.

This SHALL hold against a third-party OPC UA server as well as against the project's own
simulators: both implementations SHALL produce the same outcome and reason category when run
against the reference server of `opcua-reference-server-interop`.

#### Scenario: Same suite, both implementations
- **WHEN** the secured OPC UA e2e and conformance suites run with `IMPL=rust` and with `IMPL=c`
- **THEN** both runs pass without implementation-specific skips for security

#### Scenario: Same outcome against a third-party server
- **WHEN** the interop suite runs with `IMPL=rust` and with `IMPL=c` against the same reference
  server endpoints
- **THEN** each device reaches the same link status and the same reason category in both runs
