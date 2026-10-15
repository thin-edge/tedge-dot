## ADDED Requirements

### Requirement: Config builder produces valid connector configs
The site SHALL offer a config builder that runs entirely in the browser. It SHALL let a visitor
choose a protocol, fill in the `[connector]` settings, devices and points, and emit TOML that it
validates live against `doc/contract/schemas/config.schema.json`. Because that schema leaves the
protocol-specific tables (`connection`, `protocol_address`, a point's `address`) to the connector
specs, the builder SHALL also check the types of the keys it knows in those tables, from its own
protocol shapes (`site/src/data/protocols.json`), while keeping any other key. A schema error
SHALL be shown on the field it concerns. The output SHALL be copyable and downloadable. The builder SHALL say
that schema validation is structural, and point to `tedge-dot describe` for full validation.

#### Scenario: Valid config emitted
- **WHEN** a visitor picks `modbus`, adds one TCP device and two holding-register points
- **THEN** the builder shows TOML that validates against the config schema, with a copy and a download action

#### Scenario: Invalid field flagged
- **WHEN** a visitor enters a non-numeric value for a device's port
- **THEN** the field shows the schema error, and the copy action warns that the config is invalid

### Requirement: Config builder loads the shipped configs
The config builder SHALL be able to start from any file in `demo/config/`. CI SHALL check that
every file in `demo/config/` and `packaging/config/` validates against the schema and the
builder's protocol shapes, that every library in `demo/points.d/` validates against the
point-library schema, and that each config survives the builder's parse → emit round trip with
the same meaning.

#### Scenario: Start from the demo
- **WHEN** a visitor chooses "start from demo: opcua"
- **THEN** the form is filled from `demo/config/opcua.toml`

#### Scenario: Shipped config breaks the round trip
- **WHEN** a change to `packaging/config/snmp.toml` no longer round-trips through the builder
- **THEN** the docs check fails and names the file

### Requirement: Package picker
The site SHALL offer a package picker that recommends `tedge-dot-rs` or `tedge-dot-c` from the
visitor's answers (architecture, distribution or glibc age, protocols, features, footprint),
and explains the recommendation. It SHALL use the same package data that the install guide
renders.

#### Scenario: Old gateway
- **WHEN** a visitor answers armhf and Debian 9
- **THEN** the picker recommends `tedge-dot-c` and gives the glibc 2.17 floor as the reason

#### Scenario: PROFIBUS
- **WHEN** a visitor says they need PROFIBUS-DP
- **THEN** the picker recommends `tedge-dot-c`, or building `tedge-dot-rs` from source with the `profibus` feature

### Requirement: Report-policy playground matches the SDKs
The site SHALL offer a playground that applies a `report` policy (on_change, deadband,
min_interval, max_interval, debounce) to a chosen signal and charts the readings, the published
samples and the heartbeats. Its policy engine SHALL pass every vector in
`doc/contract/test-vectors/report/vectors.json` in CI.

#### Scenario: Deadband on a noisy signal
- **WHEN** a visitor applies `deadband = 0.5` to a noisy signal
- **THEN** the chart marks only the readings that differ from the last published value by more than 0.5 as published

#### Scenario: Vector added without engine update
- **WHEN** a new vector is added to `vectors.json` that the playground engine does not satisfy
- **THEN** the docs check fails and names the vector
