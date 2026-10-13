# moss-docs

Reads one PDF and prints one JSON result; the deriver of a moss plugin that shows a folder's PDFs as web pages. Output is deterministic for the same bytes and the same binary.

Build: `scripts/build.sh` fetches the pinned pdfium (`vendor.toml`), builds, and places `libpdfium.dylib` beside `target/release/moss-docs`. Tests: `scripts/fetch-pdfium.sh && cargo test --release`. Fixtures are regenerated with `scripts/make-fixtures.py` (needs Pillow and pypdf).

Protocol: stdin `{"path": "/abs/file.pdf"}`; stdout one line, either `{"kind":"derived","fragments":{"text":...},"facts":{"title":...,"description":"<n> pages[, scanned|, partly scanned]"}}` or `{"kind":"skip","reason":...,"message":...}`; any other failure is a non-zero exit with a one-line reason on stderr. Pdfium is found next to the executable, else `MOSS_DOCS_PDFIUM`; tesseract next to the executable, else `MOSS_DOCS_TESSERACT`, else PATH, and its absence is a failure only when a page needs OCR.

Page markers: from page 2 on, each page start in the text is marked with a markdown link to that page of the source file, `[p. 3](report%202019.pdf#page=3)` (the base name, percent-encoded as one URL path segment). The marker opens the paragraph or list item that starts the page; before a heading or table it is a paragraph of its own; inline, with a space on each side, where a paragraph or item runs across the page break (it is then one block; a word hyphenated across the break is repaired). A page without text still gets its marker. When any page was read by OCR, the text begins with a `> [!note]` block saying so.

Development aid: if `MOSS_DOCS_OCR_CACHE` names an existing directory, each page's raw Tesseract output is cached there as `<sha256 of the page image>.txt` and read back on the next run. Unset, nothing is cached.
