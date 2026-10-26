mod extract;
mod front;
mod layout;
mod ocr;

use layout::Content;
use pdfium_render::prelude::*;
use std::io::{Read, Write};
use std::path::PathBuf;

const MIN_TEXT_CHARS: usize = 20;

fn fail(msg: &str) -> ! {
    eprintln!("moss-docs: {msg}");
    std::process::exit(1);
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn skip(reason: &str, message: &str) -> ! {
    println!("{{\"kind\":\"skip\",\"reason\":{},\"message\":{}}}", js(reason), js(message));
    std::process::exit(0);
}

fn pdfium_path() -> Option<PathBuf> {
    let name = Pdfium::pdfium_platform_library_name();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        let c = dir.join(&name);
        if c.is_file() {
            return Some(c);
        }
    }
    std::env::var_os("MOSS_DOCS_PDFIUM").map(PathBuf::from).filter(|p| p.is_file())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("moss-docs {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    std::panic::set_hook(Box::new(|i| {
        let m = i.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| i.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        eprintln!("moss-docs: internal error: {}", m.lines().next().unwrap_or(""));
    }));
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        fail("could not read stdin");
    }
    let req: serde_json::Value = serde_json::from_str(&input).unwrap_or_else(|_| fail("stdin is not valid JSON"));
    let path = req.get("path").and_then(|p| p.as_str()).unwrap_or_else(|| fail("missing \"path\" in request"));
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(&format!("cannot read input file: {e}")));

    let lib = pdfium_path().unwrap_or_else(|| fail("libpdfium not found next to the executable or in MOSS_DOCS_PDFIUM"));
    let bindings = Pdfium::bind_to_library(&lib).unwrap_or_else(|e| fail(&format!("cannot load pdfium: {e}")));
    let pdfium = Pdfium::new(bindings);

    let doc = match pdfium.load_pdf_from_byte_slice(&bytes, None) {
        Ok(d) => d,
        Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => skip(
            "password-protected",
            "This PDF is password-protected, so its text can't be read. Save a copy without the password to show its text here.",
        ),
        Err(_) => skip(
            "damaged",
            "This file can't be opened as a PDF. It may be damaged; try saving it again from the program that made it.",
        ),
    };

    let meta_title = doc
        .metadata()
        .get(PdfDocumentMetadataTagType::Title)
        .map(|t| t.value().to_string());

    // Pass 1: text layer per page; OCR the pages that have none.
    let mut contents: Vec<Content> = Vec::new();
    let mut text_layer_chars = 0usize;
    let mut tess: Option<Option<PathBuf>> = None;
    let tmp = tempfile::tempdir().unwrap_or_else(|e| fail(&format!("no temp dir: {e}")));
    if doc.pages().len() == 0 {
        skip("damaged", "This file can't be opened as a PDF. It may be damaged; try saving it again from the program that made it.");
    }
    for i in 0..doc.pages().len() {
        let Ok(page) = doc.pages().get(i) else {
            contents.push(Content::Ocr(String::new()));
            continue;
        };
        let lp = extract::page_lines(&page);
        let n: usize = lp.lines.iter().map(|l| l.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
        text_layer_chars += n;
        if n >= MIN_TEXT_CHARS {
            contents.push(Content::Lines(lp));
            continue;
        }
        let t = tess.get_or_insert_with(ocr::find_tesseract);
        let Some(t) = t.as_ref() else { fail("tesseract not found") };
        match ocr::ocr_page(&page, t, tmp.path()) {
            Ok(raw) => {
                let m: usize = raw.chars().filter(|c| !c.is_whitespace()).count();
                if m >= MIN_TEXT_CHARS || n == 0 {
                    contents.push(Content::Ocr(raw));
                } else {
                    contents.push(Content::Lines(lp));
                }
            }
            Err(e) => fail(&e),
        }
    }

    let file_name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let r = layout::render(contents, meta_title, &file_name);
    if r.body_chars < MIN_TEXT_CHARS && text_layer_chars == 0 {
        skip("no readable text", "No text could be read from this PDF, even by recognizing its scanned pages.");
    }
    let title = match &r.title {
        Some(t) => js(t),
        None => "null".into(),
    };
    let date = r.date.as_ref().map(|d| format!(",\"date\":{}", js(d))).unwrap_or_default();
    let line = format!("{{\"kind\":\"derived\",\"fragments\":{{\"text\":{}}},\"facts\":{{\"title\":{},\"description\":{}{}}}}}\n", js(&r.markdown), title, js(&r.description), date);
    let mut so = std::io::stdout().lock();
    so.write_all(line.as_bytes()).unwrap_or_else(|_| fail("could not write stdout"));
}
