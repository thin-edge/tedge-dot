## ADDED Requirements

### Requirement: Declared queue size
The OPC UA connector SHALL accept an optional `queue_size` key in `[connection]`, in
`device.protocol_address` and in `point.address`. Both the Rust and the C build SHALL accept it
at every level. The value SHALL be an integer from 1 to 65535. Both builds SHALL refuse any other
value, or a value of another type, with the same message naming the place and `queue_size`.

#### Scenario: Accepted at every level
- **WHEN** a configuration sets `queue_size = 8` in `[connection]`, `queue_size = 4` in one device's `protocol_address` and `queue_size = 32` in one point's `address`
- **THEN** both builds accept the file

#### Scenario: Out of range
- **WHEN** a point sets `address = { node_id = "ns=2;s=T", queue_size = 0 }`
- **THEN** both builds refuse the file with the same message naming the point and `queue_size`

#### Scenario: Wrong type
- **WHEN** `[connection]` sets `queue_size = "16"`
- **THEN** both builds refuse the file with the same message naming `[connection]` and `queue_size`

### Requirement: Effective queue size
Each subscribed point's effective queue size SHALL be the first one set among the point's
`address.queue_size`, its device's `protocol_address.queue_size` and `[connection] queue_size`.
When none is set it SHALL be `16`. Both builds SHALL resolve the same value for the same file.

#### Scenario: Default
- **WHEN** no level sets `queue_size`
- **THEN** every subscribed point's effective queue size is 16

#### Scenario: Point beats device beats connection
- **WHEN** `[connection] queue_size = 2`, a device sets `queue_size = 4`, and one of its points sets `queue_size = 1`
- **THEN** that point's effective queue size is 1, the device's other points use 4, and points of other devices use 2

### Requirement: Monitored items request the queue size
The OPC UA connector SHALL create each monitored item with `queueSize` equal to the effective queue
size and with `discardOldest = true`. When several points share one monitored item, the item
SHALL use the largest effective queue size among them. When the server revises the queue size to
a different value, the connector SHALL log one `info` message naming the device, the point, the
requested size and the revised size, and SHALL keep the item as granted.

#### Scenario: Every change of a fast tag arrives
- **WHEN** a subscribed node changes 5 times per publishing interval and the point's effective queue size is 16
- **THEN** both builds publish a sample for each of the 5 changes, in order

#### Scenario: Latest value only with queue size 1
- **WHEN** a subscribed node changes 5 times per publishing interval and the point sets `queue_size = 1`
- **THEN** both builds publish only the latest of the 5 values per publishing interval

#### Scenario: Server revises the queue size
- **WHEN** a point requests a queue size of 16 and the server revises it to 1
- **THEN** an `info` log names the device, the point, 16 requested and 1 revised, and the point stays subscribed
