# moss-docs

Reads one PDF and prints one JSON result; the deriver of a moss plugin that shows a folder's PDFs as web pages. Output is deterministic for the same bytes and the same binary.

Build: `scripts/build.sh` fetches the pinned pdfium (`vendor.toml`), builds, and places `libpdfium.dylib` beside `target/release/moss-docs`. Tests: `scripts/fetch-pdfium.sh && cargo test --release`. Fixtures are regenerated with `scripts/make-fixtures.py` (needs Pillow and pypdf).

Protocol: stdin `{"path": "/abs/file.pdf"}`; stdout one line, either `{"kind":"derived","fragments":{"text":...},"facts":{"title":...}}` or `{"kind":"skip","reason":...,"message":...}`; any other failure is a non-zero exit with a one-line reason on stderr. Pdfium is found next to the executable, else `MOSS_DOCS_PDFIUM`; tesseract next to the executable, else `MOSS_DOCS_TESSERACT`, else PATH, and its absence is a failure only when a page needs OCR.
