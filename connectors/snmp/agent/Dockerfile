# SNMP agent simulator (pysnmp): the device the SNMP connector polls and writes to. Seeds, communities
# and v3 users are documented at the top of agent.py. Build context is the repository root.
#
#   SNMP_AGENT_PORT   UDP port to serve on (default 161; the demo runs it host-networked on 1161)
#   SNMP_ENGINE_ID    this agent's SNMPv3 engine ID, hex (default 80001f8880e2e0000000000001)
ARG BASE_REGISTRY=public.ecr.aws/docker/library
FROM ${BASE_REGISTRY}/python:3.12-slim
COPY connectors/snmp/agent/requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir -r /tmp/requirements.txt
COPY connectors/snmp/agent/agent.py /agent.py
ENV PYTHONUNBUFFERED=1 SNMP_AGENT_PORT=161
EXPOSE 161/udp
# Healthy once the agent answers its own sysDescr.0 over v2c (pysnmp's client, no net-snmp needed).
HEALTHCHECK --interval=3s --timeout=3s --retries=20 CMD ["python3", "/agent.py", "--check"]
CMD ["python3", "/agent.py"]
