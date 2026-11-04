#!/bin/sh
# Runs the command Nandi passes (the CLI's ACP command) after putting Codex's
# config into the home directory, which is an empty tmpfs on every start.
set -e
mkdir -p "$HOME/.codex"
cp /opt/nandi/codex-config.toml "$HOME/.codex/config.toml"
exec "$@"
