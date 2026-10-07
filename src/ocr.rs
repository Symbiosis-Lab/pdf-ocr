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
    if let Ok(p) = std::env::var("MOSS_DOCS_TESSERACT") {
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

/// Render the page near 300 dpi (capped so huge sheets stay tractable), OCR it.
pub fn ocr_page(page: &PdfPage, tesseract: &Path, tmp: &Path) -> Result<String, String> {
    let (w, h) = (page.width().value, page.height().value);
    let mut scale = 300.0 / 72.0;
    let longest = w.max(h) * scale;
    if longest > 6000.0 {
        scale *= 6000.0 / longest;
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
    cmd.arg(&png).arg("-").args(["-l", "eng", "--psm", "3", "--dpi", &dpi]);
    cmd.env("OMP_THREAD_LIMIT", "1");
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("tessdata"))) {
        if d.is_dir() {
            cmd.env("TESSDATA_PREFIX", d);
        }
    }
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|e| format!("tesseract failed to run: {e}"))?;
    let _ = std::fs::remove_file(&png);
    if !out.status.success() {
        return Err("tesseract exited with an error".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).replace('\u{c}', "").replace("\r\n", "\n"))
}
