//! End-to-end tests: run the built binary on the synthetic fixtures.
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn pdfium() -> String {
    let p = format!("{}/vendor/libpdfium.dylib", env!("CARGO_MANIFEST_DIR"));
    assert!(std::path::Path::new(&p).exists(), "run scripts/fetch-pdfium.sh first");
    p
}

fn run_stdin(stdin: &str) -> Output {
    let cache = tempfile::tempdir().unwrap();
    run_stdin_cached(stdin, cache.path())
}

fn run_stdin_cached(stdin: &str, cache: &std::path::Path) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pdf-ocr"))
        .env("PDF_OCR_PDFIUM", pdfium())
        .env("PDF_OCR_CACHE", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(name: &str) -> Output {
    run_stdin(&format!("{{\"path\":{},\"future\":1}}", serde_json::to_string(&fixture(name)).unwrap()))
}

fn json(o: &Output) -> Value {
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8(o.stdout.clone()).unwrap();
    assert_eq!(s.matches('\n').count(), 1, "exactly one line");
    serde_json::from_str(&s).unwrap()
}

#[test]
fn version() {
    let o = Command::new(env!("CARGO_BIN_EXE_pdf-ocr")).arg("--version").output().unwrap();
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("pdf-ocr {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn text_pdf_markdown() {
    let o = run("text3.pdf");
    let raw = String::from_utf8(o.stdout.clone()).unwrap();
    assert!(raw.starts_with("{\"kind\":\"derived\",\"fragments\":{\"text\":"), "key order: {raw}");
    assert!(raw.trim_end().ends_with("\"facts\":{\"title\":\"Audit of Example Program Controls\",\"description\":\"3 pages\"}}"));
    let j = json(&o);
    let t = j["fragments"]["text"].as_str().unwrap();
    // plain text: no heading syntax, the title is not repeated
    assert!(t.contains("[p. 2](text3.pdf#page=2) Corrective Action Plan\n\n"), "{t}");
    assert!(!t.lines().any(|l| l.starts_with('#')));
    assert!(!t.contains("Audit of Example Program Controls"));
    // wrapped lines joined, hyphenation repaired
    assert!(t.contains("responsibility for approvals"));
    // list items are plain paragraph text, their markers escaped
    assert!(t.contains("Assign a single office"));
    assert!(!t.lines().any(|l| l.starts_with("- ") || l.starts_with("1. ")), "{t}");
    // numbered items "Corrective Action #N" stay in the text
    assert!(t.contains("Corrective Action #1:") && t.contains("Corrective Action #2:"));
    // running header/footer and page numbers stripped
    for gone in ["Confidential Draft", "Example Agency Audit Division", "Page 1 of 3", "Page 3 of 3"] {
        assert!(!t.contains(gone), "{gone} should be stripped:\n{t}");
    }
}

#[test]
fn deterministic() {
    let a = run("text3.pdf").stdout;
    let b = run("text3.pdf").stdout;
    assert_eq!(a, b);
}

#[test]
fn encrypted_is_skip() {
    let j = json(&run("encrypted.pdf"));
    assert_eq!(j["kind"], "skip");
    assert_eq!(j["reason"], "password-protected");
    assert!(j["message"].as_str().unwrap().contains("password-protected"));
}

#[test]
fn not_a_pdf_is_skip() {
    let j = json(&run("notpdf.pdf"));
    assert_eq!(j["kind"], "skip");
    assert_eq!(j["reason"], "damaged");
}

#[test]
fn bad_requests_fail_cleanly() {
    for input in ["", "not json", "{}", "{\"path\":\"/nonexistent/x.pdf\"}"] {
        let o = run_stdin(input);
        assert!(!o.status.success(), "{input}");
        assert!(o.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&o.stderr).trim().lines().count(), 1);
    }
}

fn have_tesseract() -> bool {
    Command::new("tesseract").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
        || std::path::Path::new("/opt/homebrew/bin/tesseract").exists()
}

#[test]
fn scanned_page_is_ocred() {
    if !have_tesseract() {
        eprintln!("NOTE: tesseract not installed; skipping OCR test");
        return;
    }
    let a = run("scan1.pdf");
    let j = json(&a);
    let t = j["fragments"]["text"].as_str().unwrap();
    assert!(t.contains("building permit application"), "{t}");
    assert_eq!(j["facts"]["title"], "scan1");
    assert_eq!(a.stdout, run("scan1.pdf").stdout, "OCR output must be deterministic");
}

#[test]
fn side_column_comes_after_the_main_column() {
    let j = json(&run("sidebar.pdf"));
    let t = j["fragments"]["text"].as_str().unwrap();
    let pos = |x: &str| t.find(x).unwrap_or_else(|| panic!("{x} missing:\n{t}"));
    // the letter reads straight through, sidebar names never land inside its lines
    assert!(t.contains("Dear Chair Example:"));
    assert!(!t.lines().any(|l| l.contains("Jane Roe") && l.contains("March")), "{t}");
    assert!(pos("March 3, 2021") < pos("Jane Roe"));
    assert!(pos("Purpose") < pos("Jane Roe"));
    assert!(t.contains("\n\nPurpose\n\n"), "{t}");
}

fn text_of(name: &str) -> (String, Value) {
    let j = json(&run(name));
    (j["fragments"]["text"].as_str().unwrap().to_string(), j["facts"].clone())
}

const NOTE_ALL: &str = "> [!note]\n> This text was read by machine from a scan and can contain errors. The PDF is the document of record; a page number opens it at that page.\n\n";

#[test]
fn page_markers_open_the_paragraph_that_starts_a_page() {
    let (t, facts) = text_of("heading3.pdf");
    assert!(t.contains("records.\n\n[p. 2](heading3.pdf#page=2) Findings\n\n"), "{t}");
    // page 3 begins with a paragraph: the marker is the first thing in it
    assert!(t.contains("\n\n[p. 3](heading3.pdf#page=3) Recommendations follow"), "{t}");
    assert!(!t.contains("page=1"), "page 1 gets no marker:\n{t}");
    assert_eq!(facts["title"], "Overview");
    assert_eq!(facts["description"], "3 pages");
    assert!(!t.contains("[!note]"));
}

#[test]
fn paragraph_running_across_a_page_break_is_one_block_with_an_inline_marker() {
    let (t, facts) = text_of("para_break.pdf");
    // "admin-" / "istrative" is repaired, the marker follows the repaired word
    assert!(t.contains("approved by an administrative [p. 2](para_break.pdf#page=2) office that had no written authority"), "{t}");
    assert_eq!(t.split("\n\n").count(), 2, "joined paragraph + the second paragraph:\n{t}");
    assert!(t.starts_with("The review covered"), "{t}");
    assert_eq!(facts["description"], "2 pages");
}

#[test]
fn capitalised_page_start_is_not_joined() {
    let (t, _) = text_of("new_para.pdf");
    assert!(t.contains("following section\n\n[p. 2](new_para.pdf#page=2) The next section describes"), "{t}");
}

#[test]
fn file_name_is_percent_encoded_in_markers() {
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("rapport (é) 2019.pdf");
    std::fs::copy(fixture("heading3.pdf"), &dst).unwrap();
    let j = json(&run_stdin(&format!("{{\"path\":{}}}", serde_json::to_string(dst.to_str().unwrap()).unwrap())));
    let t = j["fragments"]["text"].as_str().unwrap();
    assert!(t.contains("\n\n[p. 2](rapport%20%28%C3%A9%29%202019.pdf#page=2) Findings"), "{t}");
    assert!(t.contains("[p. 3](rapport%20%28%C3%A9%29%202019.pdf#page=3) Recommendations"), "{t}");
}

#[test]
fn scanned_document_has_note_markers_joins_and_description() {
    if !have_tesseract() {
        eprintln!("NOTE: tesseract not installed; skipping OCR test");
        return;
    }
    let (t, facts) = text_of("scan2.pdf");
    assert!(t.starts_with(NOTE_ALL), "{t}");
    assert!(t.contains("will send a letter in the [p. 2](scan2.pdf#page=2) coming weeks to say what happens next."), "{t}");
    assert!(!t.contains("page=1"));
    assert_eq!(facts["title"], "scan2", "neither the note nor a marker may become the title: {facts}");
    assert_eq!(facts["description"], "2 pages, scanned");
}

#[test]
fn one_page_scan_note_drops_the_page_clause() {
    if !have_tesseract() {
        return;
    }
    let (t, facts) = text_of("scan1.pdf");
    assert!(t.starts_with("> [!note]\n> This text was read by machine from a scan and can contain errors. The PDF is the document of record.\n\n"), "{t}");
    assert!(!t.contains("#page="), "{t}");
    assert_eq!(facts["description"], "1 page, scanned");
}

#[test]
fn partly_scanned_document() {
    if !have_tesseract() {
        return;
    }
    let (t, facts) = text_of("mixed.pdf");
    assert!(t.starts_with("> [!note]\n> Parts of this text were read by machine from scanned pages and can contain errors. The PDF is the document of record; a page number opens it at that page.\n\n"), "{t}");
    assert!(t.contains("[p. 2](mixed.pdf#page=2) Scanned Letter to the Commissioner"), "{t}");
    assert_eq!(facts["description"], "2 pages, partly scanned");
    assert_eq!(facts["title"], "Overview");
}

#[test]
fn page_without_text_still_gets_its_marker() {
    if !have_tesseract() {
        return;
    }
    let (t, facts) = text_of("blank_mid.pdf");
    assert!(t.contains("needed.\n\n[p. 2](blank_mid.pdf#page=2)\n\n[p. 3](blank_mid.pdf#page=3) Gamma page text"), "{t}");
    assert!(!t.contains("[!note]"), "a blank page is not evidence of a scan");
    assert_eq!(facts["description"], "3 pages");
}

fn cached_run(name: &str, cache: &std::path::Path) -> String {
    let o = run_stdin_cached(&format!("{{\"path\":{}}}", serde_json::to_string(&fixture(name)).unwrap()), cache);
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn entries(cache: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(cache).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect()
}

#[test]
fn extraction_cache_is_read_back_and_never_changes_the_output() {
    let cache = tempfile::tempdir().unwrap();
    let cold = cached_run("text3.pdf", cache.path());
    let files = entries(cache.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap().to_str().unwrap().len(), 64 + ".json".len());
    assert_eq!(cached_run("text3.pdf", cache.path()), cold);
    // the entry is what the next run reads: plant a title only the cache holds
    let mut e: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    e["pages"] = serde_json::json!([{ "ocr": "Words that only the cache holds, long enough to count as text.\n" }]);
    std::fs::write(&files[0], e.to_string()).unwrap();
    assert!(cached_run("text3.pdf", cache.path()).contains("Words that only the cache holds"));
    // another PDF is a different key
    cached_run("para_break.pdf", cache.path());
    assert_eq!(entries(cache.path()).len(), 2);
}

#[test]
fn corrupt_extraction_entry_is_read_again_and_rewritten() {
    let cache = tempfile::tempdir().unwrap();
    let cold = cached_run("text3.pdf", cache.path());
    let file = entries(cache.path()).remove(0);
    std::fs::write(&file, "{ not json").unwrap();
    assert_eq!(cached_run("text3.pdf", cache.path()), cold);
    assert!(serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).is_ok());
}

#[test]
fn unusable_cache_location_does_not_fail_the_derivation() {
    let d = tempfile::tempdir().unwrap();
    let blocker = d.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    let cold = cached_run("text3.pdf", d.path());
    assert_eq!(cached_run("text3.pdf", &blocker.join("sub")), cold);
}
