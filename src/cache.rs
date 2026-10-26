//! The durable cache of the expensive stage: what was read from a PDF, before any shaping.
//!
//! An entry holds each page's text-layer lines or OCR text, the metadata title and the
//! text-layer character count: everything `layout::render` needs and nothing it
//! produces. It is keyed by the PDF's bytes and the extraction fingerprint, so a change
//! to shaping (`layout.rs`, `front.rs`, escaping, titles, dates) reuses entries.

use crate::layout::{Content, Line, Page};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Bump this whenever the cached structure changes or the code that fills it
/// (`extract.rs`, `ocr.rs`, the OCR-or-text decision in `main.rs`) would produce
/// different content for the same PDF and engines. Never bump it for shaping code.
pub const EXTRACT_FORMAT_VERSION: u32 = 1;

pub const DEFAULT_CAP: u64 = 2 << 30;

pub struct Extraction {
    pub meta_title: Option<String>,
    pub text_layer_chars: usize,
    pub contents: Vec<Content>,
}

pub fn key(pdf: &[u8], fingerprint: &str) -> String {
    sha256_hex(format!("{}\0v{EXTRACT_FORMAT_VERSION}\0{fingerprint}", sha256_hex(pdf)).as_bytes())
}

/// `$MOSS_CACHE_DIR/extract` (the directory moss gives a plugin, when it can be
/// written), else `$PDF_OCR_CACHE`, else the platform cache directory. None
/// when no home is known.
pub fn default_dir() -> Option<PathBuf> {
    dir_from(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
}

fn dir_from(var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = var("MOSS_CACHE_DIR").map(|d| d.join("extract")) {
        if std::fs::create_dir_all(&dir).is_ok() {
            return Some(dir);
        }
    }
    if let Some(d) = var("PDF_OCR_CACHE") {
        return Some(d);
    }
    let base = if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Caches")
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|h| h.join(".cache")))?
    };
    Some(base.join("pdf-ocr/extract"))
}

fn entry_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.json"))
}

/// A missing, unreadable or malformed entry is a miss. A hit is touched so the sweep keeps it.
pub fn get(dir: &Path, key: &str) -> Option<Extraction> {
    let path = entry_path(dir, key);
    let ex = from_json(&serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?)?;
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(ex)
}

/// Written to a temp file and renamed, so a reader never sees half an entry.
pub fn put(dir: &Path, key: &str, ex: &Extraction, cap: u64) -> std::io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let result = std::fs::write(&tmp, to_json(ex).to_string()).and_then(|_| std::fs::rename(&tmp, entry_path(dir, key)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sweep(dir, cap);
    Ok(())
}

/// Delete the least recently used files until the directory fits `cap` bytes.
pub fn sweep(dir: &Path, cap: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

fn to_json(ex: &Extraction) -> Value {
    let pages: Vec<Value> = ex
        .contents
        .iter()
        .map(|c| match c {
            Content::Ocr(t) => json!({ "ocr": t }),
            Content::Lines(p) => {
                let lines: Vec<Value> = p.lines.iter().map(|l| json!([l.col, l.x0, l.x1, l.base, l.size, l.bold, l.text])).collect();
                json!({ "height": p.height, "lines": lines })
            }
        })
        .collect();
    json!({ "title": ex.meta_title, "chars": ex.text_layer_chars, "pages": pages })
}

fn from_json(v: &Value) -> Option<Extraction> {
    let f = |v: &Value| v.as_f64().map(|x| x as f32);
    let mut contents = Vec::new();
    for p in v.get("pages")?.as_array()? {
        if let Some(t) = p.get("ocr") {
            contents.push(Content::Ocr(t.as_str()?.to_string()));
            continue;
        }
        let mut lines = Vec::new();
        for l in p.get("lines")?.as_array()? {
            let l = l.as_array()?;
            lines.push(Line {
                col: u8::try_from(l.first()?.as_u64()?).ok()?,
                x0: f(l.get(1)?)?,
                x1: f(l.get(2)?)?,
                base: f(l.get(3)?)?,
                size: f(l.get(4)?)?,
                bold: l.get(5)?.as_bool()?,
                text: l.get(6)?.as_str()?.to_string(),
            });
        }
        contents.push(Content::Lines(Page { height: f(p.get("height")?)?, lines }));
    }
    let meta_title = match v.get("title")? {
        Value::Null => None,
        t => Some(t.as_str()?.to_string()),
    };
    Some(Extraction { meta_title, text_layer_chars: usize::try_from(v.get("chars")?.as_u64()?).ok()?, contents })
}

pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample() -> Extraction {
        let line = |text: &str, base: f32| Line { x0: 72.0, x1: 540.25, base, size: 11.1, bold: base > 700.0, text: text.into(), col: 1 };
        Extraction {
            meta_title: Some("Annual Report".into()),
            text_layer_chars: 1234,
            contents: vec![
                Content::Lines(Page { height: 792.0, lines: vec![line("Heading", 720.5), line("Body \"text\" \u{2026}", 700.1)] }),
                Content::Ocr("scanned words\nmore".into()),
                Content::Ocr(String::new()),
            ],
        }
    }

    fn same(a: &Extraction, b: &Extraction) -> bool {
        to_json(a) == to_json(b)
    }

    #[test]
    fn location_prefers_the_moss_cache_dir_then_the_docs_override_then_the_home_default() {
        let tmp = std::env::temp_dir().join(format!("pdf-ocr-dir-order-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let blocked = tmp.join("a-file");
        std::fs::write(&blocked, "x").unwrap();
        fn env(pairs: &[(&'static str, PathBuf)]) -> impl Fn(&str) -> Option<PathBuf> {
            let pairs = pairs.to_vec();
            move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone())
        }
        let moss = ("MOSS_CACHE_DIR", tmp.join("moss"));
        let docs = ("PDF_OCR_CACHE", PathBuf::from("/docs"));
        let home = ("HOME", PathBuf::from("/home/u"));
        assert_eq!(dir_from(env(&[moss.clone(), docs.clone(), home.clone()])), Some(tmp.join("moss/extract")));
        assert_eq!(dir_from(env(&[docs.clone(), home.clone()])), Some(PathBuf::from("/docs")));
        let unwritable = ("MOSS_CACHE_DIR", blocked);
        assert_eq!(dir_from(env(&[unwritable, docs, home.clone()])), Some(PathBuf::from("/docs")));
        assert!(dir_from(env(&[home])).unwrap().ends_with("pdf-ocr/extract"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn round_trips_exactly() {
        let ex = sample();
        let back = from_json(&serde_json::from_str(&to_json(&ex).to_string()).unwrap()).unwrap();
        assert!(same(&ex, &back));
        let Content::Lines(p) = &back.contents[0] else { panic!() };
        assert_eq!((p.lines[1].base, p.lines[1].size), (700.1, 11.1));
    }

    #[test]
    fn hit_returns_what_was_stored() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "k", &sample(), DEFAULT_CAP).unwrap();
        assert!(same(&get(d.path(), "k").unwrap(), &sample()));
    }

    #[test]
    fn key_follows_pdf_and_engine() {
        let k = key(b"pdf", "pdfium=1|tesseract 5.5.2");
        assert_eq!(k, key(b"pdf", "pdfium=1|tesseract 5.5.2"));
        assert_ne!(k, key(b"other pdf", "pdfium=1|tesseract 5.5.2"));
        assert_ne!(k, key(b"pdf", "pdfium=2|tesseract 5.5.2"));
        assert_ne!(k, key(b"pdf", "pdfium=1|tesseract 5.5.3"));
    }

    #[test]
    fn miss_for_absent_other_key_or_missing_dir() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a", &sample(), DEFAULT_CAP).unwrap();
        assert!(get(d.path(), "b").is_none());
        assert!(get(&d.path().join("nope"), "a").is_none());
    }

    #[test]
    fn corrupt_entry_is_a_miss_and_is_rewritten() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["", "{", "{\"title\":null}", "{\"title\":null,\"chars\":1,\"pages\":[{\"lines\":[[1]]}]}"] {
            std::fs::write(entry_path(d.path(), "k"), bad).unwrap();
            assert!(get(d.path(), "k").is_none(), "{bad}");
        }
        put(d.path(), "k", &sample(), DEFAULT_CAP).unwrap();
        assert!(same(&get(d.path(), "k").unwrap(), &sample()));
    }

    #[test]
    fn sweep_deletes_oldest_first_and_hits_refresh() {
        let d = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for (name, age) in [("old", 300), ("mid", 200), ("new", 100)] {
            let p = d.path().join(name);
            std::fs::write(&p, [0u8; 10]).unwrap();
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(now - Duration::from_secs(age)).unwrap();
        }
        sweep(d.path(), 25);
        assert!(!d.path().join("old").exists() && d.path().join("mid").exists() && d.path().join("new").exists());
        sweep(d.path(), 10);
        assert!(!d.path().join("mid").exists() && d.path().join("new").exists());

        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a", &sample(), DEFAULT_CAP).unwrap();
        let p = entry_path(d.path(), "a");
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(now - Duration::from_secs(1000)).unwrap();
        get(d.path(), "a").unwrap();
        assert!(std::fs::metadata(&p).unwrap().modified().unwrap() > now - Duration::from_secs(10));
    }

    #[test]
    fn io_errors_are_errors_not_panics() {
        let d = tempfile::tempdir().unwrap();
        let blocker = d.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        assert!(put(&blocker.join("sub"), "k", &sample(), DEFAULT_CAP).is_err());
        assert!(get(&blocker, "k").is_none());
        sweep(&blocker.join("sub"), 0);
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(&[b'a'; 1000]), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    }
}
