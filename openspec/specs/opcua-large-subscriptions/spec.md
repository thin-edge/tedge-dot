# opcua-large-subscriptions Specification

## Purpose
TBD - created by archiving change load-test-performance. Update Purpose after archive.
## Requirements
### Requirement: Monitored items are created in batches
The OPC UA connector SHALL create a device's monitored items in requests of at most 500 items
each, in both builds. A device SHALL be able to subscribe however many points it has: none of its
requests or replies may carry more items than that limit.

#### Scenario: Large device subscribes
- **WHEN** a device has 5000 subscribed points on a server that accepts them
- **THEN** both builds arm all 5000 monitored items in 10 requests, the link is `connected`, and a change to any of the nodes is published

#### Scenario: Small device unchanged
- **WHEN** a device has 20 subscribed points
- **THEN** its monitored items are created in one request

### Requirement: Large replies are decoded
The Rust OPC UA client SHALL be configured to decode arrays of at least 65535 elements, so that a
device with more than 1000 points is not refused with `BadDecodingError`.

#### Scenario: More than 1000 points read
- **WHEN** a Rust connector polls or subscribes a device with 5000 points
- **THEN** no `BadDecodingError` occurs and every point delivers a sample

### Requirement: Batching keeps each build's failure rule
Batching SHALL NOT change what a refused item does to a device.
- In the Rust build, any refused item or failed batch SHALL delete the device's subscription,
  including the items of earlier batches, and keep all the device's points polled.
- In the C build, a refused item or a failed batch SHALL leave only the points concerned on the
  polling schedule. The subscription SHALL be torn down only when no item at all was armed.

#### Scenario: Rust rejects in the second batch
- **WHEN** a Rust connector subscribes 800 points and one item of the second batch is refused
- **THEN** the subscription is deleted, the error names the refused point, and all 800 points stay polled

#### Scenario: C rejects in the second batch
- **WHEN** a C connector subscribes 800 points and one item of the second batch is refused
- **THEN** 799 points are delivered by push and the refused one stays polled

### Requirement: Grouping is linear in the number of points
The Rust OPC UA connector SHALL group points that watch the same node and element into one
monitored item in time linear in the number of points.

#### Scenario: Many points subscribe quickly
- **WHEN** a device with 30,000 subscribed points on distinct nodes subscribes
- **THEN** grouping the points takes no more than linear time (no per-point search through the items created so far)

