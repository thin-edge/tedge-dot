# SNMP notification simulator: net-snmp's own snmptrap/snmpinform, so the connector is tested
# against an encoder it did not write. Build context is the repository root.
#
# Sends nothing by itself unless TRAP_INTERVAL is set (seconds): the e2e suite triggers every
# notification explicitly with `send-trap`, the demo lets it send a round periodically.
# With FORWARD_TO=<host>:<port> it is instead a trap FORWARDER: net-snmp snmptrapd receiving on
# udp/162 and forwarding everything there (see entrypoint.sh).
#
# (The agent the connector polls is a separate image: connectors/snmp/agent.)
ARG BASE_REGISTRY=public.ecr.aws/docker/library
FROM ${BASE_REGISTRY}/debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends snmp snmptrapd \
    && rm -rf /var/lib/apt/lists/*
COPY connectors/snmp/sim/send-trap /usr/local/bin/send-trap
COPY connectors/snmp/sim/entrypoint.sh /entrypoint.sh
RUN chmod +x /usr/local/bin/send-trap /entrypoint.sh
# Numeric OIDs only: no MIBs to load, and no MIB type checks rejecting the enterprise OIDs.
ENV MIBS="" MIBDIRS=/nonexistent TRAP_TARGET=connector:162 TRAP_INTERVAL=0
HEALTHCHECK --interval=3s --timeout=2s --retries=10 CMD command -v snmptrap >/dev/null
ENTRYPOINT ["/entrypoint.sh"]
