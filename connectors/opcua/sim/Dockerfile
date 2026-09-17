# OPC-UA simulator (python-asyncua) for the e2e harness.
# Base images come from the AWS ECR Public mirror of the Docker Official
# Images rather than Docker Hub: GitHub-hosted runners share Docker Hub's
# anonymous pull quota, and CI has hit auth.docker.io connection resets
# mid-build. Override BASE_REGISTRY to go back to docker.io/library.
ARG BASE_REGISTRY=public.ecr.aws/docker/library

FROM ${BASE_REGISTRY}/python:3.12-slim
WORKDIR /app
COPY connectors/opcua/sim/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY connectors/opcua/sim/server.py .
# The secured mode generates its certificates with the shared PKI vector generator.
COPY connectors/opcua/conformance/tools/genpki.py .
EXPOSE 4840
CMD ["python", "server.py"]
