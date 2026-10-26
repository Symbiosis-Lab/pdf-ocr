//! Rasterize with pdfium, recognise with Tesseract.

use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn find_tesseract() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    if let Some(dir) = exe.as_ref().and_then(|e| e.parent()) {
        let c = dir.join(if cfg!(windows) { "tesseract.exe" } else { "tesseract" });
        if c.is_file() {
            return Some(c);
        }
    }
    if let Ok(p) = std::env::var("PDF_OCR_TESSERACT") {
        if Path::new(&p).is_file() {
            return Some(PathBuf::from(p));
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("tesseract"))
        .find(|c| c.is_file())
        .or_else(|| {
            let h = PathBuf::from("/opt/homebrew/bin/tesseract");
            h.is_file().then_some(h)
        })
}

const DPI: f32 = 300.0;
const MAX_SIDE_PX: f32 = 6000.0;
const LANG: &str = "eng";
const PSM: &str = "3";

fn bundled_tessdata() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.join("tessdata")).filter(|d| d.is_dir())
}

/// Everything about the OCR engine that can change what it reads: the tesseract
/// version, the installed language codes, the content of the traineddata file in
/// use, and the settings above. No path goes in, so a plugin version that unpacks
/// into a new directory keeps its readings.
pub fn fingerprint() -> String {
    let Some(t) = find_tesseract() else { return "tesseract=none".into() };
    let run = |arg: &str| {
        let mut cmd = Command::new(&t);
        cmd.arg(arg).stdin(Stdio::null()).stderr(Stdio::null());
        if let Some(d) = bundled_tessdata() {
            cmd.env("TESSDATA_PREFIX", d);
        }
        cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    compose(&run("--version"), &run("--list-langs"))
}

/// `--list-langs` opens with a line naming the tessdata directory, then one code per line.
fn compose(version: &str, list_langs: &str) -> String {
    let mut lines = list_langs.lines();
    let dir = lines.next().and_then(|l| l.split('"').nth(1));
    let mut codes: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    codes.sort_unstable();
    let data = dir
        .and_then(|d| std::fs::read(Path::new(d).join(format!("{LANG}.traineddata"))).ok())
        .map_or("none".into(), |b| crate::cache::sha256_hex(&b));
    format!(
        "{}|langs={}|{LANG}.traineddata={data}|lang={LANG} psm={PSM} dpi={DPI} max_side={MAX_SIDE_PX}",
        version.lines().take(2).collect::<Vec<_>>().join(" "),
        codes.join(",")
    )
}

/// Render the page near 300 dpi (capped so huge sheets stay tractable), OCR it.
pub fn ocr_page(page: &PdfPage, tesseract: &Path, tmp: &Path) -> Result<String, String> {
    let (w, h) = (page.width().value, page.height().value);
    let mut scale = DPI / 72.0;
    let longest = w.max(h) * scale;
    if longest > MAX_SIDE_PX {
        scale *= MAX_SIDE_PX / longest;
    }
    let img = page
        .render_with_config(&PdfRenderConfig::new().scale_page_by_factor(scale))
        .map_err(|e| format!("render failed: {e}"))?
        .as_image()
        .map_err(|e| format!("bitmap failed: {e}"))?
        .into_luma8();
    let png = tmp.join("page.png");
    img.save(&png).map_err(|e| format!("png save failed: {e}"))?;
    let dpi = (scale * 72.0).round().to_string();
    let mut cmd = Command::new(tesseract);
    cmd.arg(&png).arg("-").args(["-l", LANG, "--psm", PSM, "--dpi", &dpi]);
    cmd.env("OMP_THREAD_LIMIT", "1");
    if let Some(d) = bundled_tessdata() {
        cmd.env("TESSDATA_PREFIX", d);
    }
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|e| format!("tesseract failed to run: {e}"))?;
    let _ = std::fs::remove_file(&png);
    if !out.status.success() {
        return Err("tesseract exited with an error".into());
    }
    Ok(clean(&String::from_utf8_lossy(&out.stdout)))
}

fn clean(raw: &str) -> String {
    raw.replace('\u{c}', "").replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::compose;

    fn bundle(tag: &str, traineddata: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("pdf-ocr-fp-{}-{tag}", std::process::id())).join("bundle/tessdata");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("eng.traineddata"), traineddata).unwrap();
        format!("List of available languages in \"{}/\" (2):\neng\nosd\n", dir.display())
    }

    #[test]
    fn same_bundle_in_two_directories_has_one_fingerprint() {
        let (a, b) = (bundle("a", b"model"), bundle("b", b"model"));
        assert_ne!(a, b);
        assert_eq!(compose("tesseract 5.5.0\n", &a), compose("tesseract 5.5.0\n", &b));
    }

    #[test]
    fn changed_traineddata_bytes_change_the_fingerprint() {
        let (a, b) = (bundle("c", b"model"), bundle("d", b"model2"));
        assert_ne!(compose("tesseract 5.5.0\n", &a), compose("tesseract 5.5.0\n", &b));
    }

    #[test]
    fn no_path_leaks_into_the_fingerprint() {
        assert!(!compose("v\n", &bundle("e", b"m")).contains("pdf-ocr-fp"));
    }

    #[test]
    fn fingerprint_text_is_fixed_and_names_no_program() {
        let sha = crate::cache::sha256_hex(b"model");
        let want = format!(
            "tesseract 5.5.3  leptonica|langs=eng,osd|eng.traineddata={sha}|lang=eng psm=3 dpi=300 max_side=6000"
        );
        assert_eq!(compose("tesseract 5.5.3\n leptonica\n", &bundle("f", b"model")), want);
    }
}
