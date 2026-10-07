# Corpus evaluation

Run on 2026-10-07 over a corpus of public agency records (373 PDFs, 8,187 pages, about 700 MB, not committed) with `scripts/eval-corpus.py`. Full outputs live outside the repo.

## Outcome counts

- derived: 372
- skip: 1 (a one-page conference handout with no text and no image, so "no readable text" is right)
- failure (non-zero exit): 0
- title null: 269 of 372 derived. Nearly all are scanned documents, where OCR gives no font sizes; moss falls back to the filename.

## Time

Measured one document at a time on an Apple-silicon Mac, tesseract single-threaded.

- text layer: about 4 ms per page (33-page report in 0.12 s, 26-page designed report in 1.2 s)
- scanned page, 300 dpi render plus tesseract: 1.3 to 1.6 s per page (5-page scan 6.3 s, 6-page scan 9.6 s)
- the corpus took 3,810 CPU-seconds in total, run six documents in parallel; the longest single document (a 114-page scan with large images) took 175 s

Determinism: two runs of the same scan produced identical bytes (md5 compared), as do two runs of a text PDF; both are asserted in `tests/cli.rs`. Tesseract output is only identical for the same Tesseract build and language data, so the cache key should include the packaged tesseract version.

## What was checked by eye

About fifteen outputs: meeting agendas and minutes, audit letters, two scanned audit resolutions, two designed annual reports, a Local Law 13 report, a conference slide deck, a 129-page text audit with an appended policy. Wrapped lines, de-hyphenation, numbered and bulleted lists, bold headings, and header/footer removal behave as intended. Fixes made because of the corpus: word spacing threshold, bold detection by font name, learned paragraph gap for double-spaced minutes, "5t h" raised ordinals, dot leaders in contents pages, letter-spaced display type, hyphen kept in "federal-level" when both halves are words used elsewhere in the document, repeated first/last lines removed from OCR pages, roman-numeral section markers.

## Known defects

1. Column detection handles one clean vertical gutter per page (a narrow sidebar goes after the main column; two comparable prose columns read left then right). Pages with several gutters, a gutter crossed by running text, or sidebars made of short fragments are still read row by row, and the two-column meeting agendas are not fixed because their cells are too short to count as prose.
2. Body-font section titles (short, title-case or all-caps, no final punctuation, directly above a paragraph of 80+ characters) are promoted, at most six per page. This also promotes some people's names and form labels that sit above a paragraph (about a dozen per hundred documents), and in slide decks every bold line still becomes a heading. Heading levels are ranked by size within a document, so a body-font title can land at `###` when the document has a larger heading elsewhere.
3. OCR output has no geometry, so scanned documents get no headings, no title, and stray marks from stamps and signatures ("- al") appear as paragraphs or one-item lists.
4. Text rotated off the horizontal is dropped on purpose (watermarks, margin stamps), which also drops all text of a page that is itself stored rotated.
5. Chart axis labels and table numbers come out as one short paragraph each.

## Packaging notes

- pdfium: `libpdfium.dylib` from bblanchon/pdfium-binaries `chromium/8086` (sha256 in `vendor.toml`) sits next to the executable; one build per platform (`pdfium-mac-arm64`, `-mac-x64`, `-linux-x64`, `-win-x64`). The binary has no other native dependency.
- tesseract: Homebrew's tesseract is dynamically linked against leptonica, libpng, libtiff and so on, so copying only the executable does not work. Ship either a relocatable bundle (executable plus its dylibs, rewritten with `install_name_tool -change ... @executable_path/`, plus a `tessdata/eng.traineddata` next to it; the binary sets `TESSDATA_PREFIX` to that directory when it exists) or a static build per platform. Language data is about 4 MB for `eng` (fast) and 15 MB for `eng` (best); the corpus was run with Homebrew's default data.
- OCR work is capped by rasterising at no more than 6,000 pixels on the long side, so oversized sheets are recognised at under 300 dpi.

## Second pass (side columns, section titles)

Re-run over all 373 PDFs: 372 derived, 1 skip, 0 failures; output is byte-identical across repeated runs. Headings (`##` to `####`) across the corpus rose from 5,162 to about 7,000, mostly the standard sections of audit determinations (Purpose, Scope, Methodology, Next Steps).
