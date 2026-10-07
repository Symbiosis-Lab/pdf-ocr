//! pdfium -> positioned text lines.

use crate::layout::{normalize_char, Line, Page};
use pdfium_render::prelude::*;

struct Ch {
    s: String,
    space: bool,
    x0: f32,
    x1: f32,
    base: f32,
    size: f32,
    bold: bool,
}

struct Seg {
    x0: f32,
    x1: f32,
    base: f32,
    size: f32,
    bold: bool,
    text: String,
    n: usize,
}

fn is_bold(c: &PdfPageTextChar) -> bool {
    // pdfium's weight is unreliable for embedded subsets, so look at the name too.
    let by_weight = match c.font_weight() {
        Some(PdfFontWeight::Weight600) | Some(PdfFontWeight::Weight700Bold) | Some(PdfFontWeight::Weight800) | Some(PdfFontWeight::Weight900) => true,
        Some(PdfFontWeight::Custom(w)) => w >= 600,
        _ => false,
    };
    by_weight || {
        let n = c.font_name().to_lowercase();
        n.contains("bold") || n.contains("black") || n.contains("heavy") || n.contains("demi") || n.ends_with("-bd") || n.contains("-bd")
    }
}

fn page_chars(page: &PdfPage) -> Vec<Ch> {
    let mut out = Vec::new();
    let Ok(text) = page.text() else { return out };
    for c in text.chars().iter() {
        if c.is_generated().unwrap_or(false) {
            continue;
        }
        let Some(mut u) = c.unicode_char() else { continue };
        if u == '\u{2}' {
            u = '-'; // pdfium reports a line-end hyphen as U+0002
        }
        if u == '\r' || u == '\n' || u == '\t' || u == '\u{2}' || u.is_control() {
            continue;
        }
        let ang = c.angle_degrees().unwrap_or(0.0).rem_euclid(360.0);
        if ang > 2.0 && ang < 358.0 {
            continue; // rotated text: watermarks, margin stamps
        }
        let space = u.is_whitespace();
        let Ok(b) = c.loose_bounds() else { continue };
        let Ok(base) = c.origin_y() else { continue };
        let size = c.scaled_font_size().value;
        if size <= 0.5 {
            continue;
        }
        let s = match normalize_char(u) {
            Some(r) => r.to_string(),
            None => u.to_string(),
        };
        let bold = if space { false } else { is_bold(&c) };
        out.push(Ch { s, space: space || s_is_space(&u), x0: b.left().value, x1: b.right().value, base: base.value, size, bold });
    }
    out
}

fn s_is_space(c: &char) -> bool {
    c.is_whitespace()
}

fn finish_seg(chars: &[Ch]) -> Option<Seg> {
    let mut text = String::new();
    let mut pending = false;
    let mut prev_x1: Option<f32> = None;
    let (mut x0, mut x1) = (f32::MAX, f32::MIN);
    let mut sizes: Vec<(i32, usize)> = Vec::new();
    let (mut bold_n, mut n) = (0usize, 0usize);
    let mut base_acc = Vec::new();
    for c in chars {
        if c.space {
            pending = true;
            continue;
        }
        if let Some(px) = prev_x1 {
            if pending || c.x0 - px > 0.15 * c.size {
                if !text.is_empty() {
                    text.push(' ');
                }
            }
        }
        pending = false;
        text.push_str(&c.s);
        prev_x1 = Some(c.x1);
        x0 = x0.min(c.x0);
        x1 = x1.max(c.x1);
        let k = (c.size * 10.0).round() as i32;
        match sizes.iter_mut().find(|(s, _)| *s == k) {
            Some(e) => e.1 += 1,
            None => sizes.push((k, 1)),
        }
        n += 1;
        if c.bold {
            bold_n += 1;
        }
        base_acc.push(c.base);
    }
    if n == 0 {
        return None;
    }
    let size = sizes.iter().max_by_key(|(k, c)| (*c, -*k)).map(|(k, _)| *k as f32 / 10.0).unwrap();
    base_acc.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let base = base_acc[base_acc.len() / 2];
    Some(Seg { x0, x1, base, size, bold: bold_n * 2 > n, text, n })
}

fn rows_to_lines(mut segs: Vec<Seg>, col: u8) -> Vec<Line> {
    segs.sort_by(|a, b| b.base.partial_cmp(&a.base).unwrap().then(a.x0.partial_cmp(&b.x0).unwrap()));
    let mut rows: Vec<Vec<Seg>> = Vec::new();
    for s in segs {
        match rows.last_mut() {
            Some(r) if (r[0].base - s.base).abs() <= 0.35 * r[0].size.max(s.size) => r.push(s),
            _ => rows.push(vec![s]),
        }
    }
    let mut lines = Vec::new();
    for mut r in rows {
        r.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap());
        let total: usize = r.iter().map(|s| s.n).sum();
        let big = r.iter().max_by_key(|s| s.n).unwrap();
        let (size, base) = (big.size, big.base);
        let bold = r.iter().filter(|s| s.bold).map(|s| s.n).sum::<usize>() * 2 > total;
        let mut text = String::new();
        let mut tabular = false;
        let mut px1: Option<f32> = None;
        for s in &r {
            if let Some(p) = px1 {
                let gap = s.x0 - p;
                if gap > 2.5 * size {
                    tabular = true;
                    text.push('\u{2003}');
                } else if gap > 0.15 * size || !text.is_empty() {
                    text.push(' ');
                }
            }
            text.push_str(&s.text);
            px1 = Some(s.x1);
        }
        let mut text = collapse_leaders(&fix_ordinals(text.trim()));
        // letter-spaced display type: "P O L I C Y"
        let toks: Vec<&str> = text.split(' ').collect();
        if toks.len() >= 4 && toks.iter().all(|t| t.chars().count() == 1 && t.chars().all(|c| c.is_alphabetic())) {
            text = toks.concat();
        }
        if text.is_empty() {
            continue;
        }
        lines.push(Line { col, x0: r[0].x0, x1: r.last().unwrap().x1, base, size, bold, text, tabular });
    }
    lines
}

/// Split a page into reading-order column groups when one clean vertical
/// gutter runs down it. A narrow side column (letterhead sidebar) comes after
/// the main column; two comparable columns read left then right. Needs prose
/// on both sides, so table columns (short cells) are left alone.
fn split_columns(segs: Vec<Seg>, width: f32) -> Vec<Vec<Seg>> {
    let prose = |v: &[&Seg]| -> usize { v.iter().filter(|s| s.text.chars().count() >= 20 || s.n >= 20).count() };
    let mut cands: Vec<f32> = segs.iter().map(|s| s.x1).collect();
    cands.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cands.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    let mut best: Option<(f32, f32)> = None; // (gutter width, split x)
    for &g in &cands {
        if g < width * 0.12 || g > width * 0.88 {
            continue;
        }
        let left: Vec<&Seg> = segs.iter().filter(|s| s.x1 <= g + 0.5).collect();
        let right: Vec<&Seg> = segs.iter().filter(|s| s.x0 > g + 0.5).collect();
        let cross = segs.len() - left.len() - right.len();
        if cross * 20 > segs.len() || left.len() < 4 || right.len() < 4 {
            continue;
        }
        let lmax = left.iter().map(|s| s.x1).fold(f32::MIN, f32::max);
        let rmin = right.iter().map(|s| s.x0).fold(f32::MAX, f32::min);
        let gap = rmin - lmax;
        let lc: usize = left.iter().map(|s| s.n).sum();
        let rc: usize = right.iter().map(|s| s.n).sum();
        let sidebar = lc.min(rc) * 3 < lc.max(rc);
        let ok = if sidebar { prose(&left).min(prose(&right)).max(0) >= 1 || lc.min(rc) >= 40 } else { prose(&left) >= 4 && prose(&right) >= 4 };
        if gap >= 14.0 && ok && best.is_none_or(|(bg, _)| gap > bg) {
            best = Some((gap, (lmax + rmin) / 2.0));
        }
    }
    let Some((_, x)) = best else { return vec![segs] };
    let (mut l, mut r, mut cross) = (Vec::new(), Vec::new(), Vec::new());
    for s in segs {
        if s.x1 <= x { l.push(s) } else if s.x0 >= x { r.push(s) } else { cross.push(s) }
    }
    r.extend(cross); // a stray spanning run belongs with the wider side
    let lc: usize = l.iter().map(|s| s.n).sum();
    let rc: usize = r.iter().map(|s| s.n).sum();
    if lc * 3 < rc { vec![r, l] } else { vec![l, r] }
}

pub fn page_lines(page: &PdfPage) -> Page {
    let height = page.height().value;
    let chars = page_chars(page);
    // runs: consecutive chars on one baseline moving rightwards
    let mut segs: Vec<Seg> = Vec::new();
    let mut run: Vec<Ch> = Vec::new();
    let mut run_base = 0.0f32;
    for c in chars {
        let split = match run.iter().rev().find(|p| !p.space) {
            Some(p) => {
                !c.space && ((c.base - run_base).abs() > 0.35 * c.size.max(p.size) || c.x0 < p.x1 - 1.5 * c.size)
            }
            None => false,
        };
        if split {
            if let Some(s) = finish_seg(&run) {
                segs.push(s);
            }
            run.clear();
        }
        if run.is_empty() || run.iter().all(|p| p.space) {
            if !c.space {
                run_base = c.base;
            }
        }
        run.push(c);
    }
    if let Some(s) = finish_seg(&run) {
        segs.push(s);
    }
    let width = page.width().value;
    let mut lines = Vec::new();
    for (i, g) in split_columns(segs, width).into_iter().enumerate() {
        lines.extend(rows_to_lines(g, i as u8));
    }
    Page { height, lines }
}

/// Dot leaders in a table of contents ("Chapter ......... 5") become an ellipsis.
fn collapse_leaders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0usize;
    let mut pending = String::new();
    for c in s.chars() {
        if c == '.' || (c == ' ' && run > 0) {
            if c == '.' {
                run += 1;
            }
            pending.push(c);
            continue;
        }
        if run >= 4 {
            out.truncate(out.trim_end().len());
            out.push_str(" … ");
        } else {
            out.push_str(&pending);
        }
        pending.clear();
        run = 0;
        out.push(c);
    }
    if run >= 4 {
        out.truncate(out.trim_end().len());
        out.push_str(" …");
    } else {
        out.push_str(&pending);
    }
    out
}

/// A raised "th" often comes out as "t h" next to its number: "285t h" -> "285th".
fn fix_ordinals(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        out.push(cs[i]);
        if cs[i].is_ascii_digit() && i + 3 < cs.len() + 0 && cs[i + 2] == ' ' {
            let pair = (cs[i + 1], cs[i + 3]);
            let after = cs.get(i + 4).copied();
            if matches!(pair, ('t', 'h') | ('s', 't') | ('n', 'd') | ('r', 'd')) && after.map_or(true, |c| !c.is_alphanumeric()) {
                out.push(pair.0);
                out.push(pair.1);
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinals() {
        assert_eq!(super::fix_ordinals("285t h Meeting, March 5t h, 2026"), "285th Meeting, March 5th, 2026");
        assert_eq!(super::fix_ordinals("1 s test"), "1 s test");
        assert_eq!(super::collapse_leaders("About the Commission ......... 5"), "About the Commission … 5");
        assert_eq!(super::collapse_leaders("Etc. and so..."), "Etc. and so...");
    }
}
