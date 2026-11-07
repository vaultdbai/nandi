# Nandi connectors: where published MCP connectors run. Node LTS (npx),
# Python and uv (uvx), and nothing else: no coding CLIs, no credentials.
# A connector is a pinned package (`npx <package>@<version>`,
# `uvx <package>==<version>`), fetched through the egress proxy when its
# container starts. It reaches only its connector.md hosts.
#
# Build: docker build -t ghcr.io/vaultdbai/nandi-connectors:sandbox-5 runtimes/nandi-connectors (or pull it)
# Nandi runs `docker run -i --rm --init --read-only ... --entrypoint npx ghcr.io/vaultdbai/nandi-connectors:sandbox-5 -y <package>@<version>`
# on the internal nandi-agents network. The image also carries the egress
# relay (relay.mjs) for the nandi-egress container.
FROM node:22.20.0-bookworm-slim

# Checked by the app before it starts a container (runtime.rs SANDBOX_VERSION)
LABEL org.nandi.sandbox="5"

ARG UV_VERSION=0.9.5

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3 python3-pip python3-venv \
    && pip3 install --no-cache-dir --break-system-packages "uv==${UV_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

# Package caches live in the home tmpfs (the root filesystem is read-only
# at run time) and die with the container
ENV NO_UPDATE_NOTIFIER=1 \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NPM_CONFIG_FUND=false \
    UV_NO_PROGRESS=1

COPY relay.mjs /opt/nandi/

USER node
WORKDIR /home/node
