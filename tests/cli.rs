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
    let mut c = Command::new(env!("CARGO_BIN_EXE_moss-docs"))
        .env("MOSS_DOCS_PDFIUM", pdfium())
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
    let o = Command::new(env!("CARGO_BIN_EXE_moss-docs")).arg("--version").output().unwrap();
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("moss-docs {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn text_pdf_markdown() {
    let o = run("text3.pdf");
    let raw = String::from_utf8(o.stdout.clone()).unwrap();
    assert!(raw.starts_with("{\"kind\":\"derived\",\"fragments\":{\"text\":"), "key order: {raw}");
    assert!(raw.trim_end().ends_with("\"facts\":{\"title\":\"Audit of Example Program Controls\"}}"));
    let j = json(&o);
    let t = j["fragments"]["text"].as_str().unwrap();
    // headings: start at ##, title is not repeated
    assert!(t.contains("\n### Corrective Action Plan\n") || t.contains("## Corrective Action Plan"));
    assert!(t.starts_with("## ") || t.starts_with("### "));
    assert!(!t.contains("Audit of Example Program Controls"));
    // sentence-length lines are never headings
    assert!(!t.lines().any(|l| l.starts_with('#') && l.len() > 100));
    // wrapped lines joined, hyphenation repaired
    assert!(t.contains("responsibility for approvals"));
    // lists preserved
    assert!(t.contains("1. Assign a single office"));
    assert!(t.contains("3. Train staff"));
    assert!(t.contains("- Several files lacked"));
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
    assert!(j["facts"]["title"].is_null());
    assert_eq!(a.stdout, run("scan1.pdf").stdout, "OCR output must be deterministic");
}
