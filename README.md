# moss-docs

Reads one PDF and prints one JSON result; the deriver of a moss plugin that shows a folder's PDFs as web pages. Output is deterministic for the same bytes and the same binary.

Install into a moss site: `scripts/install.sh <site>` copies `plugin/manifest.json`, which names this version's release archive by URL and sha256; moss downloads and unpacks it into `~/.moss/stacks/` the first time the site is built, then asks once before running it. Releases are on [GitHub](https://github.com/Symbiosis-Lab/moss-docs/releases); today there is one build, macOS on Apple silicon (`darwin-arm64`). Each archive carries moss-docs, pdfium and Tesseract with its libraries and English language data, so no Tesseract needs to be installed, and the licences of all of them in `licenses/`.

Release: `scripts/build.sh`, then `python3 -I scripts/package-macos.py` (relocates Homebrew's Tesseract into the archive and checks nothing still loads from Homebrew), then `python3 -I scripts/release-manifest.py` (writes `plugin/manifest.json` with the archive's sha256); upload `dist/*.tar.gz` to the release `v<version>` and commit the manifest. For a local build instead, `scripts/install-dev.sh <site>` points a site at `target/release/`.

Build: `scripts/build.sh` fetches the pinned pdfium (`vendor.toml`), builds, and places `libpdfium.dylib` beside `target/release/moss-docs`. Tests: `scripts/fetch-pdfium.sh && cargo test --release`. Fixtures are regenerated with `scripts/make-fixtures.py` (needs Pillow and pypdf).

Protocol: stdin `{"path": "/abs/file.pdf"}`; stdout one line, either `{"kind":"derived","fragments":{"text":...},"facts":{"title":...,"description":"<n> pages[, scanned|, partly scanned]"}}` or `{"kind":"skip","reason":...,"message":...}`; any other failure is a non-zero exit with a one-line reason on stderr. Pdfium is found next to the executable, else `MOSS_DOCS_PDFIUM`; tesseract next to the executable, else `MOSS_DOCS_TESSERACT`, else PATH, and its absence is a failure only when a page needs OCR.

Text: the text is plain paragraphs. Lists, headings and tables are not inferred; wrapped lines are joined into paragraphs, running headers and page numbers are dropped, and everything extracted or OCR'd is backslash-escaped so it renders as the literal text (a `- - - -` leader rule, a leading `#` or `1.`, `*stars*`, `[brackets]`). The title fact comes from the largest type on the first text page, else the PDF's metadata.

Page markers: from page 2 on, each page start in the text is marked with a markdown link to that page of the source file, `[p. 3](report%202019.pdf#page=3)` (the base name, percent-encoded as one URL path segment). The marker opens the paragraph that starts the page, or sits inline, with a space on each side, where a paragraph runs across the page break (it is then one paragraph; a word hyphenated across the break is repaired). A page without text still gets its marker. When any page was read by OCR, the text begins with a `> [!note]` block saying so.

Development aid: if `MOSS_DOCS_OCR_CACHE` names an existing directory, each page's raw Tesseract output is cached there as `<sha256 of the page image>.txt` and read back on the next run. Unset, nothing is cached.
