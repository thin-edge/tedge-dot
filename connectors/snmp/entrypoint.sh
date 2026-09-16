#!/bin/sh
# Wait for the broker, then start the connector. The simulators only send when the suite tells
# them to, so there is nothing else to wait for: the connector resolves their names itself.
set -e

echo "waiting for broker:1883 ..."
until nc -z broker 1883; do sleep 1; done

echo "starting tedge-dot"
exec /usr/bin/tedge-dot /etc/connector.toml
