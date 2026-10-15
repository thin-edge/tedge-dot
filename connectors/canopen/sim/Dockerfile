# Build the CANopen simulator container.
# Sets up vcan0, installs python-canopen, and runs the SDO server node.

# Base images come from the AWS ECR Public mirror of the Docker Official
# Images rather than Docker Hub: GitHub-hosted runners share Docker Hub's
# anonymous pull quota, and CI has hit auth.docker.io connection resets
# mid-build. Override BASE_REGISTRY to go back to docker.io/library;
# CI sets it to the GHCR mirror of .github/mirror-images.txt.
ARG BASE_REGISTRY=public.ecr.aws/docker/library

FROM ${BASE_REGISTRY}/python:3.11-slim-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        can-utils \
        iproute2 \
        kmod \
    && rm -rf /var/lib/apt/lists/*

COPY connectors/canopen/sim/requirements.txt /requirements.txt
RUN pip install --no-cache-dir -r /requirements.txt

COPY connectors/canopen/sim/simulator.py /simulator.py

# Create vcan0, load the vcan module, and start the simulator.
CMD set -e; \
    modprobe vcan || true; \
    ip link add dev vcan0 type vcan 2>/dev/null || true; \
    ip link set up vcan0; \
    echo "vcan0 is up"; \
    python3 /simulator.py
