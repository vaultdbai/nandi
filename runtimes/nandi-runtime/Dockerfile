# Nandi runtime: a sandboxed MCP tool server. It serves tools over MCP on
# stdio (docker exec) and makes no LLM calls — the host owns the agent loop.
# The image also carries the egress relay (relay.py) for the nandi-egress
# container.
#
# Build: docker build -t ghcr.io/vaultdbai/nandi-runtime:sandbox-6 runtimes/nandi-runtime (or pull it)
FROM python:3.12-slim

# Checked by the app before it starts a container (runtime.rs SANDBOX_VERSION)
LABEL org.nandi.sandbox="6"

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

# What review and ops agents expect in the shell: git and gh (from GitHub's
# signed apt repo), curl and jq. They reach the internet only through the
# egress proxy (HTTPS_PROXY) and only to the hosts skill.md allows.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git jq \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt /opt/nandi/requirements.txt
RUN pip install --no-cache-dir -r /opt/nandi/requirements.txt

# The host mounts the agent folder at /agent/skill (read-only) and a
# per-agent workdir at /agent/workdir (read-write). The root filesystem is
# read-only at run time; /tmp and /home/agent are tmpfs.
RUN useradd --uid 1000 --create-home agent \
    && mkdir -p /agent/skill /agent/workdir \
    && chown agent:agent /agent/workdir

COPY app.py relay.py /opt/nandi/

USER agent
WORKDIR /agent/workdir

# The tool server runs per connection: docker exec -i … app.py --stdio
CMD ["sleep", "infinity"]
