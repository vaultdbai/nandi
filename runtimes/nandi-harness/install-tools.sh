#!/bin/sh
# Installs a tool bundle Nandi fetches on first use (not redistributed in
# this image) into the named volume mounted at /opt/nandi-tools/<bundle>.
# Nandi runs it once per bundle version, in a one-off container:
#
#   docker run --rm ... -v nandi-tools-claude-code-<rev>:/opt/nandi-tools/claude-code \
#     ghcr.io/vaultdbai/nandi-harness:sandbox-6 /opt/nandi/install-tools.sh claude-code
#
# npm ci installs exactly the lockfile shipped in this image and checks every
# package against its integrity hash. The last line written is the marker
# Nandi checks: the lockfile's sha256, in <dest>/.nandi-ready.
set -eu
bundle="${1:?usage: install-tools.sh <bundle>}"
case "$bundle" in
    claude-code) ;;
    *) echo "Unknown tool bundle: $bundle" >&2; exit 2 ;;
esac
source_dir="/opt/nandi/$bundle"
dest="/opt/nandi-tools/$bundle"

rm -rf "$dest/node_modules" "$dest/.nandi-ready"
cp "$source_dir/package.json" "$source_dir/package-lock.json" "$dest/"
cd "$dest"
echo "Installing $bundle from its lockfile…"
npm ci --omit=dev --no-fund --no-audit --no-update-notifier \
    --cache /tmp/npm-cache --loglevel=http
# npm installs both libc builds of the CLI; this image is glibc (Debian)
rm -rf node_modules/@anthropic-ai/*-musl
sha256sum package-lock.json | cut -d' ' -f1 > .nandi-ready
echo "Installed $bundle ($(du -sh node_modules | cut -f1))."
