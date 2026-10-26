#!/bin/sh
# Install the released plugin into a moss site: copies plugin/manifest.json,
# which names the release archives; moss downloads the one for its platform
# the first time the site is built. For a local build, use install-dev.sh.
set -eu
site="${1:?usage: install.sh <site folder>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$site/.moss/plugins/pdf-ocr"
mkdir -p "$dest"
cp "$root/plugin/manifest.json" "$dest/manifest.json"
echo "installed pdf-ocr $(sed -n 's/.*"version": "\(.*\)".*/\1/p' "$root/plugin/manifest.json" | head -1) into $dest"
