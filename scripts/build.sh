#!/bin/sh
# Release build with libpdfium placed next to the binary (target/release/).
set -eu
cd "$(dirname "$0")/.."
./scripts/fetch-pdfium.sh
cargo build --release
cp vendor/libpdfium.dylib target/release/libpdfium.dylib
echo "built: $(pwd)/target/release/pdf-ocr"
