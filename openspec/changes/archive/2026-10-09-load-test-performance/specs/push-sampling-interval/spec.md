## MODIFIED Requirements

### Requirement: OPC UA monitored-item sampling
The OPC UA connector SHALL create each subscribed point's monitored item with
`samplingInterval` equal to the point's effective sampling interval in milliseconds.
`queueSize` SHALL be the point's effective queue size (see `opcua-monitored-item-queue`, default
16), and `discardOldest` SHALL stay `true`. Both builds SHALL request the same values for the
same configuration.

#### Scenario: Fast sampling with slow polling
- **WHEN** an OPC UA device sets `poll_interval = "1h"` and `sampling_interval = "200ms"`, and a subscribed node changes every 500 ms
- **THEN** both builds publish a sample for each change within about one second of it, not once an hour

#### Scenario: Default queue size
- **WHEN** no level sets `queue_size`
- **THEN** both builds create each monitored item with `queueSize = 16` and `discardOldest = true`
