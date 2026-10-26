#!/bin/sh
# Download and verify the pinned pdfium build into ./vendor/ (macOS arm64).
set -eu
cd "$(dirname "$0")/.."
TAG="chromium/8086"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$TAG/pdfium-mac-arm64.tgz"
SHA256="e98679e052c07edbb5a627980902abb823d4b3f35744d877bd21668bd9fc13ab"
mkdir -p vendor
if [ -f vendor/libpdfium.dylib ] && [ -f vendor/LICENSE ]; then echo "vendor/libpdfium.dylib present"; exit 0; fi
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/p.tgz" "$URL"
got=$(shasum -a 256 "$tmp/p.tgz" | cut -d' ' -f1)
[ "$got" = "$SHA256" ] || { echo "sha256 mismatch: $got" >&2; exit 1; }
tar -xzf "$tmp/p.tgz" -C "$tmp"
cp "$tmp/lib/libpdfium.dylib" vendor/libpdfium.dylib
# the licences ship with the release beside the library
cp "$tmp/LICENSE" vendor/LICENSE
[ -d "$tmp/licenses" ] && rm -rf vendor/licenses && cp -R "$tmp/licenses" vendor/licenses
echo "ok: vendor/libpdfium.dylib ($TAG)"
