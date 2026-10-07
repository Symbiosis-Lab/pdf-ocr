#!/bin/sh
# Install this plugin into a moss site for local use, pointing at the locally
# built binary. A release manifest names per-platform downloads instead.
set -eu
site="${1:?usage: install-dev.sh <site folder>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/moss-docs"
[ -x "$bin" ] || { echo "build first: scripts/build.sh" >&2; exit 1; }
dest="$site/.moss/plugins/moss-docs"
mkdir -p "$dest"
artifact="{ \"kind\": \"path\", \"binary\": \"$bin\" }"
sed "s|ARTIFACT|$artifact|" "$root/plugin/manifest.template.json" > "$dest/manifest.json"
echo "installed moss-docs into $dest"
