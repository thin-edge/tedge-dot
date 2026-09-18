# Spec Delta

## Purpose

Defines how the OPC UA connectors are proven to interoperate with a third-party OPC UA stack:
the OPC Foundation UA-.NETStandard reference server, packaged by the
`php-opcua/uanetstandard-test-suite` project. It covers which of the server's endpoints both
implementations must handle, how trust is bootstrapped against it, and how the suite is pinned
and gated so a young upstream project cannot destabilise the rest of CI.

## ADDED Requirements

### Requirement: Interop suite runs against the reference server
The project SHALL provide an OPC UA interop suite that runs both connector implementations
against the UA-.NETStandard reference servers. The suite SHALL be separate from the existing
e2e suites: it SHALL NOT replace `connectors/opcua/sim/server.py`, which remains the default
e2e server and the only source of the negative PKI vectors.

The suite SHALL:
- use a pinned image reference, not a floating tag, so an upstream release cannot change what
  CI proves without a deliberate commit
- publish no fixed host ports, so it runs in parallel with every other suite
- run under `IMPL=rust` and `IMPL=c` from the same suite source
- be reproducible locally through its own recipe, separate from `just test-e2e opcua`

The suite SHALL be skipped, with the reason stated in the report, when the host cannot run the
server's architecture. A skipped run SHALL NOT be reported as a pass.

#### Scenario: Suite runs in parallel with the other stacks
- **WHEN** the interop suite and the secured e2e suite run at the same time on one host
- **THEN** both complete without a port conflict

#### Scenario: Both implementations covered
- **WHEN** the interop suite runs with `IMPL=rust` and with `IMPL=c`
- **THEN** both runs execute the same tests and pass

#### Scenario: Unsupported architecture is a skip, not a pass
- **WHEN** the suite is started where the reference server image cannot run
- **THEN** the tests are reported as skipped with a reason naming the architecture

### Requirement: Every supported policy and mode connects
For each security policy the connectors support, the suite SHALL connect to a reference server
endpoint offering it and read a value. This SHALL include, on the wire and not only in
configuration validation:
- `None` with mode `none`
- `Basic256Sha256` with `sign` and with `sign_and_encrypt`
- `Aes128_Sha256_RsaOaep` with `sign_and_encrypt`
- `Aes256_Sha256_RsaPss` with `sign_and_encrypt`

For every connected device the link-status `info` SHALL report the effective `security_policy`
and `security_mode` it actually negotiated.

#### Scenario: Aes128 against a real server
- **WHEN** a device selects `Aes128_Sha256_RsaOaep` and `sign_and_encrypt` against the reference
  server's certificate endpoint
- **THEN** the device is `connected`, `info.security_policy` is `Aes128_Sha256_RsaOaep`, and a
  sample is published

#### Scenario: Sign without encryption
- **WHEN** a device selects `Basic256Sha256` with `sign` against a server that offers only
  `Sign` for that policy
- **THEN** the device is `connected` and `info.security_mode` is `sign`

### Requirement: Deprecated policies are proven on the wire
The suite SHALL connect to the reference server's legacy endpoints using `Basic128Rsa15` and
`Basic256` with `allow_deprecated_security = true`, and SHALL confirm that the same
configuration without that opt-in fails validation.

#### Scenario: Deprecated policy connects when allowed
- **WHEN** a device sets `security_policy = "Basic256"`, `security_mode = "sign_and_encrypt"`
  and `allow_deprecated_security = true` against the legacy endpoint
- **THEN** the device is `connected` and a sample is published

#### Scenario: The same device without the opt-in is refused
- **WHEN** `allow_deprecated_security` is removed from that device
- **THEN** configuration validation fails naming the device and the deprecated policy, and no
  connection is attempted

### Requirement: User identities are accepted and rejected as the server decides
The suite SHALL cover each identity type against the reference server: anonymous, username with
password, and X.509 user certificate. It SHALL also cover the rejection of each.

#### Scenario: Username accepted
- **WHEN** a device authenticates with a username and password the reference server's user
  database holds, over `sign_and_encrypt`
- **THEN** the device is `connected`

#### Scenario: Username rejected
- **WHEN** the same device uses a password the server does not accept
- **THEN** the device is `disconnected`, the reason starts with `identity rejected:`, and
  neither the retained link status nor the log contains the password

#### Scenario: X.509 user identity accepted
- **WHEN** a device presents a user certificate the reference server trusts
- **THEN** the device is `connected`

#### Scenario: Anonymous refused where the server requires a user
- **WHEN** a device connects anonymously to an endpoint that offers no anonymous token policy
- **THEN** the device is `disconnected` and the reason starts with `identity unsupported:`

### Requirement: Trust is bootstrapped through the operator flow
The reference server presents a self-signed certificate that it regenerates whenever it
starts, so the suite SHALL NOT pin a certificate ahead of time. It SHALL instead exercise the
operator flow the connectors document: the first attempt quarantines the certificate, the
operator trusts it by thumbprint with `tedge-dot pki trust`, and the device connects on a
later attempt without the connector being restarted.

The suite SHALL also cover `trust_any_server_certificate = true` against the same server.

#### Scenario: Quarantine then trust
- **WHEN** a secured device first reaches the reference server
- **THEN** the device is `disconnected` with a reason starting `certificate untrusted:`, the
  certificate is listed by `tedge-dot pki list rejected`, and after `tedge-dot pki trust
  <thumbprint>` the device becomes `connected` without a restart

#### Scenario: Opt-out connects immediately
- **WHEN** a device sets `trust_any_server_certificate = true`
- **THEN** it is `connected` on the first attempt and `info.server_certificate` is
  `not_verified`

### Requirement: A server that rejects our certificate is reported as such
The suite SHALL point a device at a reference server endpoint that does not auto-accept unknown
client certificates and has not been given the connector's certificate. The device SHALL be
`disconnected` with the reason category for the server rejecting our application certificate,
and the reason SHALL direct the operator to export the connector's certificate rather than to
change their own trust store.

After the connector's certificate is added to that server's trusted store, the device SHALL
connect.

#### Scenario: Our certificate is not trusted by the server
- **WHEN** a device connects to the strict-trust endpoint and the server has never seen the
  connector's application certificate
- **THEN** the device is `disconnected`, the reason names the connector's own certificate and
  its thumbprint, and the reason does not claim that the server's certificate is untrusted

#### Scenario: Connects once the server trusts it
- **WHEN** the connector's certificate is placed in that server's trusted store
- **THEN** the device becomes `connected`

### Requirement: Endpoints with a resource path
The reference server serves its endpoints under a resource path. The suite SHALL cover both an
endpoint with a non-empty resource path and one with none, and SHALL confirm that the connector
reaches the same server through both.

#### Scenario: Non-empty resource path
- **WHEN** a device is configured with an endpoint whose URL has a resource path
- **THEN** the device is `connected` and reads a value

#### Scenario: Empty resource path
- **WHEN** a device is configured with an endpoint whose URL has no resource path, served by
  the reference server's discovery instance
- **THEN** the device is `connected`

### Requirement: An ECC-only server is refused cleanly
The reference server offers endpoints whose policies neither connector implements. A device
pointed at such a server SHALL be `disconnected` with a reason starting `no matching endpoint:`
that names the policies the server offered. The connector SHALL keep serving its other devices
and SHALL NOT crash, hang or busy-loop.

#### Scenario: ECC endpoints produce a listed refusal
- **WHEN** a device is pointed at a server that offers only ECC policies
- **THEN** the device is `disconnected`, the reason starts with `no matching endpoint:` and
  names the ECC policies offered, and a second device on another endpoint keeps publishing

### Requirement: Values read from the reference address space
The suite SHALL read typed values from the reference server's address space and subscribe to at
least one of its changing variables, proving that both the polling and the push path work
against a third-party stack. The suite SHALL resolve namespace indices from the server's
namespace array rather than assuming a fixed index.

#### Scenario: Typed scalars read correctly
- **WHEN** the suite reads the reference server's scalar variables for each datatype the
  connector claims to support
- **THEN** each published sample carries the expected type and value

#### Scenario: Subscription delivers changes
- **WHEN** a device subscribes to a variable the reference server updates on a timer
- **THEN** samples keep arriving without the connector polling that point

#### Scenario: Namespace index is resolved, not assumed
- **WHEN** the reference server assigns its custom namespace an index other than the one it
  usually uses
- **THEN** the suite still resolves the nodes and the tests pass
