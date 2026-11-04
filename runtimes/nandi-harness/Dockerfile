# Nandi harness: the CLI harnesses agents can run in (Claude Code, Codex,
# Gemini CLI), each speaking ACP on stdio. Nothing of Nandi's own runs here:
# the CLI's model writes files and runs commands with the CLI's built-in
# tools, inside this container.
#
# Build: docker build -t ghcr.io/vaultdbai/nandi-harness:sandbox-5 runtimes/nandi-harness (or pull it)
# Nandi runs `docker run -i --rm --init ... ghcr.io/vaultdbai/nandi-harness:sandbox-5 <acp command>`
# with the agent's workdir mounted at /agent/workdir and credentials as
# environment variables of that one process.
FROM node:22.20.0-bookworm-slim

# Checked by the app before it starts a container (runtime.rs SANDBOX_VERSION)
LABEL org.nandi.sandbox="5"

# Versions Nandi is tested with (keep in step with services/harness/*.rs)
ARG CLAUDE_CODE_VERSION=2.1.302
ARG CLAUDE_AGENT_ACP_VERSION=0.93.0
ARG CODEX_VERSION=0.163.0
ARG CODEX_ACP_VERSION=0.16.0
ARG GEMINI_CLI_VERSION=0.64.0
# uv runs Python MCP servers for connectors (`uvx`)
ARG UV_VERSION=0.9.5

# What the CLIs' tools expect to find: git, ripgrep, a shell, certificates;
# and Python for connectors' MCP servers (`uvx`), which also run here
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git ripgrep procps \
        python3 python3-pip python3-venv \
    && pip3 install --no-cache-dir --break-system-packages "uv==${UV_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g --no-fund --no-audit \
        "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
        "@agentclientprotocol/claude-agent-acp@${CLAUDE_AGENT_ACP_VERSION}" \
        "@openai/codex@${CODEX_VERSION}" \
        "@zed-industries/codex-acp@${CODEX_ACP_VERSION}" \
        "@google/gemini-cli@${GEMINI_CLI_VERSION}" \
    && npm cache clean --force

# The CLIs keep per-session state in the home directory, a tmpfs (the root
# filesystem is read-only at run time) that dies with the container;
# nothing is shared between sessions
ENV DISABLE_AUTOUPDATER=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    GEMINI_CLI_DISABLE_AUTO_UPDATE=1 \
    NO_UPDATE_NOTIFIER=1

RUN mkdir -p /agent/workdir \
    && chown -R node:node /agent/workdir /home/node
# Outside the home tmpfs; the entrypoint copies it into ~/.codex
COPY codex-config.toml /opt/nandi/codex-config.toml
COPY entrypoint.sh relay.mjs /opt/nandi/
RUN chmod 0555 /opt/nandi/entrypoint.sh

USER node
WORKDIR /agent/workdir
ENTRYPOINT ["/opt/nandi/entrypoint.sh"]
