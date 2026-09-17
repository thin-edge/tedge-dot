# Spec Delta

## Purpose

Defines the on-device PKI directory that the OPC UA connectors use for their application
certificate and for server certificate trust. It also defines the `tedge-dot pki` command-line
interface that operators use to inspect and change that directory. Neither requires knowing
file-naming rules or restarting the connector.

## ADDED Requirements

### Requirement: PKI directory location
The PKI directory SHALL be set by `[connection] pki_dir`. When `pki_dir` is not set, it SHALL
default to `/var/lib/tedge-dot/opcua/pki`. A relative `pki_dir` SHALL be resolved against the
directory that contains the configuration file, never against the process working directory.
The connector SHALL create missing subdirectories only when a secured device needs them.

#### Scenario: Relative path resolves against the config file
- **WHEN** `/etc/tedge/plugins/ot/opcua.toml` sets `pki_dir = "pki"` and the connector is started from `/`
- **THEN** the connector uses `/etc/tedge/plugins/ot/pki` and creates nothing under `/`

#### Scenario: Default location
- **WHEN** a secured configuration does not set `pki_dir`
- **THEN** the connector uses `/var/lib/tedge-dot/opcua/pki`

### Requirement: PKI directory layout
Both implementations SHALL use and recognise this layout:

```
<pki_dir>/
  own/certs/        application instance certificate (DER)
  own/private/      its private key (PEM), directory mode 0700, key mode 0600
  trusted/certs/    trusted server certificates and trusted CA certificates (DER or PEM)
  trusted/crl/      CRLs of the CAs in trusted/certs (DER or PEM)
  issuers/certs/    intermediate / issuer CA certificates, not trusted on their own
  issuers/crl/      CRLs of the CAs in issuers/certs
  rejected/certs/   server certificates that failed trust validation
```

Certificate files SHALL be recognised by content. Their file names SHALL NOT matter. Files the
connector writes SHALL be named `<subject CN>_<SHA-1 thumbprint hex>.der`, with characters
that are unsafe in file names replaced. The connector SHALL write files atomically, so that
several connectors sharing one PKI directory never see a partial file and never generate two
application certificates.

A file in the PKI directory that cannot be parsed SHALL be skipped with a warning that names
the file. It SHALL NOT prevent other certificates from being used.

#### Scenario: Operator drops a PEM file with an arbitrary name
- **WHEN** the operator copies `plant-ca.crt` (PEM) into `trusted/certs/` and its CRL `plant-ca.crl` into `trusted/crl/`
- **THEN** server certificates issued by that CA are trusted on the next connect attempt

#### Scenario: Corrupt file does not break trust
- **WHEN** `trusted/certs/` contains one valid pinned certificate and one file that is not a certificate
- **THEN** the valid certificate is still trusted and a warning names the invalid file

#### Scenario: Concurrent first start
- **WHEN** two OPC UA connector configurations sharing the default PKI directory start at the same time and no application certificate exists
- **THEN** exactly one application certificate is created and both connectors use it

### Requirement: CLI entry point
Both the Rust and the C `tedge-dot` binaries SHALL provide
`tedge-dot pki <action> [options]`. The command SHALL locate the PKI directory in this order:
1. `--pki-dir <dir>`, when given.
2. `--config <file>`, whose `[connection]` settings (including `pki_dir`, `application_uri` and
   `application_name`) are used.
3. `/etc/tedge/plugins/ot/opcua.toml`, when neither option is given.
4. The default PKI location, when that file is also missing.

Every action SHALL accept `--json` for machine-readable output. An action SHALL exit with
status 0 on success, 1 on a usage or input error, and 2 when the named certificate is not
found. The command SHALL work without an MQTT broker and without reaching any OPC UA server.

#### Scenario: Works offline
- **WHEN** `tedge-dot pki list --config opcua.toml` runs on a host with no broker and no reachable server
- **THEN** it prints the PKI contents and exits 0

#### Scenario: Unknown thumbprint
- **WHEN** `tedge-dot pki trust 0123abcd` names no certificate in `rejected/certs/`
- **THEN** the command exits 2 with a message naming the thumbprint

### Requirement: Inspect and export the application certificate
`tedge-dot pki show` SHALL print the following for the application certificate:
- its path
- its subject
- its URI SAN
- its DNS and IP SANs
- its SHA-1 thumbprint
- its `notBefore` and `notAfter`
- whether the URI SAN matches the configured `application_uri`

`tedge-dot pki export [--pem] [--output <file>]` SHALL write the certificate in DER (the
default) or PEM to the file, or to standard output. It SHALL never write the private key.

#### Scenario: Show the certificate
- **WHEN** an application certificate exists and the operator runs `tedge-dot pki show --json`
- **THEN** the output contains the thumbprint, the URI SAN, `notAfter`, and a `uri_matches` boolean

#### Scenario: Export for the server administrator
- **WHEN** the operator runs `tedge-dot pki export --pem --output tedge-dot.pem`
- **THEN** `tedge-dot.pem` holds the certificate and no private key material

### Requirement: Create or renew the application certificate
`tedge-dot pki create` SHALL generate the application certificate using the same rules as the
connector's automatic generation. It SHALL accept `--application-uri`, `--hostname` (repeatable,
DNS or IP) and `--days`. When a certificate already exists, the command SHALL refuse unless
`--force` is given. With `--force`, the previous certificate and key SHALL be kept under
`own/` with a timestamp suffix rather than deleted. Files created by a command running as root
SHALL be owned by the owner of the PKI directory.

#### Scenario: Create when none exists
- **WHEN** `own/` is empty and the operator runs `tedge-dot pki create --hostname gw01.plant.local`
- **THEN** a certificate with URI SAN `application_uri` and DNS SAN `gw01.plant.local` and a private key with mode 0600 are written

#### Scenario: Refuse to overwrite
- **WHEN** a certificate exists and the operator runs `tedge-dot pki create` without `--force`
- **THEN** the command exits 1 and the existing certificate is unchanged

#### Scenario: Renew with backup
- **WHEN** the operator runs `tedge-dot pki create --force`
- **THEN** a new certificate is written, and the previous certificate and key remain under `own/` with a timestamp suffix

### Requirement: Manage server certificate trust
The CLI SHALL provide the following actions:
- `tedge-dot pki list [trusted|rejected|issuers]`: lists certificates with thumbprint, subject,
  `notAfter`, whether each is a CA, and, for CAs, whether a CRL is present. Without an argument
  it lists all three.
- `tedge-dot pki trust <thumbprint|file>`: moves a rejected certificate to `trusted/certs/`, or
  imports a DER or PEM file there.
- `tedge-dot pki reject <thumbprint>`: moves a trusted certificate to `rejected/certs/`, and
  warns when the certificate is still trusted through a CA (it must be revoked instead).
- `tedge-dot pki remove <thumbprint>`: deletes a certificate from `trusted/`, `rejected/` or
  `issuers/`.
- `tedge-dot pki add-issuer <file>`: imports a CA certificate into `issuers/certs/`.
- `tedge-dot pki add-crl <file>`: imports a CRL into the `crl/` directory next to the CA that
  issued it. It fails when that CA is in neither `trusted/` nor `issuers/`.

A thumbprint SHALL be matched case-insensitively and SHALL accept an unambiguous prefix of at
least 8 hex digits. A running connector SHALL pick up every change on its next connect attempt,
without a restart or reload.

#### Scenario: Trust a quarantined server certificate
- **WHEN** a server certificate with thumbprint `A1B2C3D4…` is in `rejected/certs/` and the operator runs `tedge-dot pki trust a1b2c3d4`
- **THEN** the certificate is moved to `trusted/certs/` and the affected device becomes `connected` on its next reconnect attempt

#### Scenario: Ambiguous prefix
- **WHEN** two certificates share the thumbprint prefix given to `trust`
- **THEN** the command exits 1 and lists both matches

#### Scenario: CRL for an unknown CA
- **WHEN** the operator runs `tedge-dot pki add-crl other.crl` and the CRL's issuer matches no CA in `trusted/` or `issuers/`
- **THEN** the command exits 1 and nothing is written

#### Scenario: Revoke trust
- **WHEN** the operator runs `tedge-dot pki reject <thumbprint>` for a pinned certificate
- **THEN** the certificate moves to `rejected/certs/` and the next reconnect of devices using that server fails with `certificate untrusted:`

### Requirement: Packaged PKI directory
The Rust and C packages SHALL create `/var/lib/tedge-dot/opcua/pki` owned by the service user
`tedge`, with mode 0750, and with `own/private` at mode 0700. They SHALL NOT generate any
certificate at install time. Removing the package SHALL NOT delete the PKI directory unless the
package is purged.

#### Scenario: Fresh install
- **WHEN** the package is installed
- **THEN** `/var/lib/tedge-dot/opcua/pki` exists, is owned by `tedge`, and contains no certificate or key

#### Scenario: Upgrade keeps trust
- **WHEN** the package is upgraded
- **THEN** existing certificates, keys and CRLs in the PKI directory are unchanged
