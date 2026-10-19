### 1. Get Linux
# Base images come from the AWS ECR Public mirror of the Docker Official
# Images rather than Docker Hub: GitHub-hosted runners share Docker Hub's
# anonymous pull quota, and CI has hit auth.docker.io connection resets
# mid-build. Override BASE_REGISTRY to go back to docker.io/library;
# CI sets it to the GHCR mirror of .github/mirror-images.txt.
ARG BASE_REGISTRY=public.ecr.aws/docker/library

FROM ${BASE_REGISTRY}/python:3.11

### 2. Install pymodbus and aiohttp
COPY connectors/modbus/sim/requirements.txt /tmp/requirements.txt
RUN pip install -r /tmp/requirements.txt

### 3. Copy modbus server configuration
COPY connectors/modbus/sim/modbus.json /modbus.json

### 3. Start server
EXPOSE 502
EXPOSE 8080    
ADD connectors/modbus/sim/simulator.py /
CMD ["python3", "simulator.py"]