# Nandi harness: the CLI harnesses agents can run in (Codex, Gemini CLI and,
# fetched on first use, Claude Code), each speaking ACP on stdio. Nothing of
# Nandi's own runs here: the CLI's model writes files and runs commands with
# the CLI's built-in tools, inside this container.
#
# Published to ghcr.io/vaultdbai/nandi-harness:sandbox-6 by
# .github/workflows/images.yml; the app pulls it. A local build tags the
# same name: docker build -t ghcr.io/vaultdbai/nandi-harness:sandbox-6 runtimes/nandi-harness
#
# Claude Code is NOT in this image: Anthropic's licence doesn't clearly allow
# redistributing it, nor the Claude Agent SDK that its ACP adapter depends
# on (the SDK bundles the Claude Code binary). The image ships only their
# lockfile (claude-code/). On first use Nandi runs install-tools.sh in a
# one-off container: `npm ci` from that lockfile into a named volume
# (nandi-tools-claude-code-*). Runs mount the volume read-only at
# /opt/nandi-tools/claude-code, whose bin is on PATH.
# See docs/developer/harness-container.md.
#
# Nandi runs `docker run -i --rm --init ... ghcr.io/vaultdbai/nandi-harness:sandbox-6 <acp command>`
# with the agent's workdir mounted at /agent/workdir and credentials as
# environment variables of that one process.
FROM node:22.20.0-bookworm-slim

# Checked by the app before it starts a container (runtime.rs SANDBOX_VERSION)
LABEL org.nandi.sandbox="6"

# Versions Nandi is tested with (keep in step with services/harness/*.rs).
# Claude Code's are in claude-code/package-lock.json
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

# Open source (Apache-2.0), so they ship in the image
RUN npm install -g --no-fund --no-audit \
        "@openai/codex@${CODEX_VERSION}" \
        "@zed-industries/codex-acp@${CODEX_ACP_VERSION}" \
        "@google/gemini-cli@${GEMINI_CLI_VERSION}" \
    && npm cache clean --force

# The CLIs keep per-session state in the home directory, a tmpfs (the root
# filesystem is read-only at run time) that dies with the container;
# nothing is shared between sessions. The tools volume's bin comes first on
# PATH; without the volume the folder is empty.
ENV DISABLE_AUTOUPDATER=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    GEMINI_CLI_DISABLE_AUTO_UPDATE=1 \
    NO_UPDATE_NOTIFIER=1 \
    PATH="/opt/nandi-tools/claude-code/node_modules/.bin:${PATH}"

# /opt/nandi-tools/claude-code is the tools volume's mount point: a new
# named volume copies this empty, node-owned folder, so the one-off install
# (as node) can write it
RUN mkdir -p /agent/workdir /opt/nandi-tools/claude-code \
    && chown -R node:node /agent/workdir /home/node /opt/nandi-tools
# Outside the home tmpfs; the entrypoint copies it into ~/.codex
COPY codex-config.toml /opt/nandi/codex-config.toml
COPY entrypoint.sh install-tools.sh relay.mjs /opt/nandi/
# What install-tools.sh installs, pinned with integrity hashes
COPY claude-code/package.json claude-code/package-lock.json /opt/nandi/claude-code/
RUN chmod 0555 /opt/nandi/entrypoint.sh /opt/nandi/install-tools.sh

USER node
WORKDIR /agent/workdir
ENTRYPOINT ["/opt/nandi/entrypoint.sh"]
