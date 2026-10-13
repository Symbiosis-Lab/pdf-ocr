//! Turn positioned text lines into markdown. Pure functions, no pdfium here.

use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Line {
    pub x0: f32,
    pub x1: f32,
    /// Baseline, PDF coordinates (larger = higher on the page).
    pub base: f32,
    pub size: f32,
    pub bold: bool,
    pub text: String,
    /// The row has wide internal gaps (table-like): keep it on its own line.
    pub tabular: bool,
    /// Reading-order column group on the page (0 for single-column pages).
    pub col: u8,
}

pub struct Page {
    pub height: f32,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Bullet,
    Numbered,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Para,
    Item(Item),
    Heading,
}

#[derive(Debug, Clone)]
struct Blk {
    kind: Kind,
    logical: Vec<String>,
    size: f32,
    bold: bool,
    ends_open: bool,
    /// Made of table-like rows: never joined across a page break.
    tabular: bool,
}

pub struct Rendered {
    pub markdown: String,
    pub title: Option<String>,
    /// "<n> pages", plus ", scanned" / ", partly scanned" when OCR was used.
    pub description: String,
    /// Non-whitespace characters of the document text itself (no note, no page markers).
    pub body_chars: usize,
}

/// One input page: its text lines, or already-recognised (OCR) markdown.
pub enum Content {
    Lines(Page),
    Ocr(String),
}

// ---------- small text helpers ----------

pub fn normalize_char(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{FB00}' => "ff",
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{00A0}' | '\u{2007}' | '\u{202F}' => " ",
        '\u{00AD}' => "",
        '\u{FFFE}' | '\u{FFFF}' | '\u{FFFD}' => "",
        _ => return None,
    })
}

fn is_page_number(s: &str) -> bool {
    let mut t = s.trim().to_lowercase();
    if let Some(r) = t.strip_prefix("page") {
        t = r.trim().to_string();
    }
    let t = t.trim_matches(|c: char| c == '-' || c == '–' || c == '—' || c.is_whitespace());
    let digits = |x: &str| !x.is_empty() && x.len() <= 4 && x.chars().all(|c| c.is_ascii_digit());
    if digits(t) {
        return true;
    }
    for sep in [" of ", "/"] {
        if let Some((a, b)) = t.split_once(sep) {
            if digits(a.trim()) && digits(b.trim()) {
                return true;
            }
        }
    }
    t.len() <= 4 && !t.is_empty() && t.chars().all(|c| "ivx".contains(c))
}

fn norm_key(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in w.chars() {
            out.extend(if c.is_ascii_digit() { '#' } else { c }.to_lowercase());
        }
    }
    out
}

fn list_marker(s: &str) -> Option<(Item, usize)> {
    let mut it = s.char_indices();
    let (_, c0) = it.next()?;
    let after = |n: usize| -> Option<usize> {
        // marker must be followed by whitespace (consumed) and more text
        let rest = &s[n..];
        let t = rest.trim_start();
        if t.is_empty() || t.len() == rest.len() {
            return None;
        }
        Some(s.len() - t.len())
    };
    if "•◦▪●■○‣►➢✓".contains(c0) || ('\u{F000}'..='\u{F0FF}').contains(&c0) {
        return after(c0.len_utf8()).map(|n| (Item::Bullet, n)).or_else(|| {
            // glyph followed directly by text
            let rest = &s[c0.len_utf8()..];
            (!rest.is_empty()).then_some((Item::Bullet, c0.len_utf8()))
        });
    }
    if matches!(c0, '-' | '–' | '—' | '*' | '·') {
        return after(c0.len_utf8()).map(|n| (Item::Bullet, n));
    }
    if c0.is_ascii_digit() {
        let nd = s.chars().take_while(|c| c.is_ascii_digit()).count();
        if nd <= 3 {
            let rest = &s[nd..];
            if rest.starts_with('.') || rest.starts_with(')') {
                return after(nd + 1).map(|n| (Item::Numbered, n));
            }
        }
        return None;
    }
    if c0.is_ascii_uppercase() {
        let n = s.chars().take_while(|c| "IVXLC".contains(*c)).count();
        if (1..=5).contains(&n) && s[n..].starts_with(". ") && s[n + 2..].chars().next().is_some_and(|c| c.is_uppercase()) {
            return after(n + 1).map(|m| (Item::Other, m));
        }
    }
    if c0.is_ascii_alphabetic() {
        let rest = &s[1..];
        if (rest.starts_with('.') && c0.is_ascii_lowercase()) || rest.starts_with(')') {
            return after(2).map(|n| (Item::Other, n));
        }
        return None;
    }
    if c0 == '(' {
        if let Some(close) = s.find(')') {
            let inner = &s[1..close];
            if !inner.is_empty()
                && inner.len() <= 4
                && (inner.chars().all(|c| c.is_ascii_digit())
                    || inner.chars().all(|c| "ivxlcdm".contains(c))
                    || (inner.len() == 1 && inner.chars().all(|c| c.is_ascii_alphabetic())))
            {
                return after(close + 1).map(|n| (Item::Other, n));
            }
        }
    }
    None
}

/// Escape characters that would change meaning in markdown.
pub fn escape_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        match c {
            '*' | '`' => {
                out.push('\\');
                out.push(c)
            }
            '_' => {
                let inner = prev.is_some_and(|p| p.is_alphanumeric()) && next.is_some_and(|n| n.is_alphanumeric());
                if !inner {
                    out.push('\\');
                }
                out.push(c)
            }
            '[' if next == Some('[') || prev == Some('[') => {
                out.push('\\');
                out.push(c)
            }
            '<' if next.is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!' || n == '?') => {
                out.push_str("\\<")
            }
            _ => out.push(c),
        }
    }
    out
}

/// Escape what matters at the start of a line (after inline escaping).
fn escape_line_start(s: &str) -> String {
    let t = s.trim_start();
    let first = t.chars().next();
    match first {
        Some('#') | Some('>') | Some('+') | Some('-') | Some('=') | Some('|') => format!("\\{t}"),
        Some(c) if c.is_ascii_digit() => {
            let nd = t.chars().take_while(|c| c.is_ascii_digit()).count();
            let rest = &t[nd..];
            if (rest.starts_with('.') || rest.starts_with(')')) && (rest.len() == 1 || rest[1..].starts_with(' ')) {
                format!("{}\\{}", &t[..nd], rest)
            } else {
                t.to_string()
            }
        }
        _ => t.to_string(),
    }
}

pub type Vocab = HashSet<String>;

fn words_of(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphabetic()).filter(|w| w.chars().count() >= 2).map(|w| w.to_lowercase())
}

fn add_vocab(v: &mut Vocab, text: &str) {
    for l in text.lines() {
        // words cut by a line-end hyphen are not evidence of how they are spelled
        let l = l.trim_end();
        let l = l.strip_suffix('-').map(|x| x.trim_end_matches(|c: char| c.is_alphabetic())).unwrap_or(l);
        v.extend(words_of(l));
    }
}

/// Join two wrapped lines. A hyphen at the break is dropped only when the
/// joined word is attested elsewhere in the document, or when the halves are
/// not words on their own ("respon-sibility"); "federal-level" keeps its hyphen.
pub fn join_wrapped(a: &mut String, b: &str, vocab: &Vocab) {
    if let Some(stem) = a.strip_suffix('-') {
        let prev_alpha = stem.chars().last().is_some_and(|c| c.is_alphabetic());
        let next = b.chars().next();
        if prev_alpha && next.is_some_and(|c| c.is_alphabetic()) {
            if next.is_some_and(|c| c.is_lowercase()) {
                let left: String = stem.chars().rev().take_while(|c| c.is_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect::<String>().to_lowercase();
                let right: String = b.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
                let joined = format!("{left}{right}");
                let halves_are_words = left.chars().count() >= 3 && right.chars().count() >= 3 && vocab.contains(&left) && vocab.contains(&right);
                if vocab.contains(&joined) || !halves_are_words {
                    a.pop();
                }
            }
            a.push_str(b);
            return;
        }
    }
    if !a.is_empty() && !b.is_empty() {
        a.push(' ');
    }
    a.push_str(b);
}

/// OCR has no geometry, so the running header/footer is whatever first or last
/// line repeats (digits normalised) on at least half of the scanned pages.
fn strip_ocr_running(pages: &mut [&mut String]) {
    let edge = |t: &str, first: bool| -> Option<usize> {
        let mut idx = t.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, _)| i);
        if first { idx.next() } else { idx.last() }
    };
    let n = pages.len();
    let need = 2.max(n.div_ceil(2));
    let mut counts: HashMap<(String, bool), usize> = HashMap::new();
    for p in pages.iter() {
        for first in [true, false] {
            if let Some(i) = edge(p, first) {
                *counts.entry((norm_key(p.lines().nth(i).unwrap()), first)).or_default() += 1;
            }
        }
    }
    for p in pages.iter_mut() {
        for first in [true, false] {
            if let Some(i) = edge(p, first) {
                let line = p.lines().nth(i).unwrap().to_string();
                let repeated = n >= 2 && counts.get(&(norm_key(&line), first)).copied().unwrap_or(0) >= need;
                if repeated || is_page_number(&line) {
                    let kept: Vec<&str> = p.lines().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| l).collect();
                    **p = kept.join("\n");
                }
            }
        }
    }
}

fn ocr_blocks(text: &str, vocab: &Vocab) -> Vec<Blk> {
    let mut blocks: Vec<Blk> = Vec::new();
    for para in text.replace(MARK, "").split("\n\n") {
        let mut cur: Vec<String> = Vec::new();
        for raw in para.lines() {
            let l = raw.trim();
            if l.is_empty() {
                continue;
            }
            if cur.is_empty() || list_marker(l).is_some() {
                cur.push(l.to_string());
            } else {
                join_wrapped(cur.last_mut().unwrap(), l, vocab);
            }
        }
        for s in cur {
            let (kind, text) = match list_marker(&s) {
                Some((Item::Bullet, n)) => (Kind::Item(Item::Bullet), s[n..].to_string()),
                Some((it, _)) => (Kind::Item(it), s),
                None => (Kind::Para, s),
            };
            let ends_open = ends_open(&text);
            blocks.push(Blk { kind, logical: vec![text], size: 0.0, bold: false, ends_open, tabular: false });
        }
    }
    blocks
}

/// A block whose last line lacks sentence-final punctuation (optionally
/// followed by closing quotes or brackets) may continue on the next page.
fn ends_open(t: &str) -> bool {
    !t.trim_end_matches(['"', '\'', '”', '’', ')', ']', '»']).ends_with(['.', '?', '!', ':', ';'])
}

// ---------- document-level pipeline ----------

fn is_generic_title(t: &str) -> bool {
    let l = t.to_lowercase();
    let words = l.split_whitespace().count();
    let org_tail = l.split_whitespace().last().is_some_and(|w| ["commission", "department", "office", "agency", "board", "city", "council"].contains(&w));
    if words <= 9 && org_tail && !l.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if words >= 7 {
        return false;
    }
    const LEAD: [&str; 12] = [
        "the city of", "city of", "state of", "county of", "office of", "department of", "board of",
        "commission", "the new york", "new york city", "new york state", "agency of",
    ];
    LEAD.iter().any(|p| l.starts_with(p))
}

pub fn meaningful_metadata_title(t: &str) -> Option<String> {
    let t = t.trim();
    let t = t.strip_prefix("Microsoft Word - ").unwrap_or(t).trim();
    let l = t.to_lowercase();
    let junk = l.ends_with(".pdf")
        || l.ends_with(".doc")
        || l.ends_with(".docx")
        || l.starts_with("untitled")
        || l.starts_with("document")
        || l.starts_with("scan");
    let n = t.chars().count();
    if junk || !(3..=150).contains(&n) || !t.chars().any(|c| c.is_alphabetic()) || t.contains('\u{FFFD}') {
        return None;
    }
    Some(t.to_string())
}

fn mode_size(pages: &[&Page]) -> f32 {
    let mut m: BTreeMap<i32, usize> = BTreeMap::new();
    for p in pages {
        for l in &p.lines {
            *m.entry((l.size * 2.0).round() as i32).or_default() += l.text.chars().filter(|c| !c.is_whitespace()).count();
        }
    }
    m.into_iter().max_by_key(|(k, n)| (*n, -*k)).map(|(k, _)| k as f32 / 2.0).unwrap_or(11.0)
}

fn strip_running(pages: &mut [&mut Page]) {
    let n = pages.len();
    // bare page numbers in the margins
    for p in pages.iter_mut() {
        let h = p.height;
        p.lines.retain(|l| {
            let from_top = h - l.base;
            !((from_top < h * 0.15 || from_top > h * 0.85) && is_page_number(&l.text))
        });
    }
    if n < 2 {
        return;
    }
    let mut seen: HashMap<(String, bool, i32), HashSet<usize>> = HashMap::new();
    let zone = |h: f32, base: f32| -> Option<(bool, i32)> {
        let from_top = h - base;
        if from_top < h * 0.15 {
            Some((true, (from_top / 6.0).round() as i32))
        } else if from_top > h * 0.85 {
            Some((false, ((h - from_top) / 6.0).round() as i32))
        } else {
            None
        }
    };
    for (pi, p) in pages.iter().enumerate() {
        for l in &p.lines {
            if let Some((top, b)) = zone(p.height, l.base) {
                seen.entry((norm_key(&l.text), top, b)).or_default().insert(pi);
            }
        }
    }
    let need = 2.max(n.div_ceil(2));
    let count = |k: &str, top: bool, b: i32| -> usize {
        let mut u: HashSet<usize> = HashSet::new();
        for d in -1..=1 {
            if let Some(s) = seen.get(&(k.to_string(), top, b + d)) {
                u.extend(s);
            }
        }
        u.len()
    };
    for p in pages.iter_mut() {
        let h = p.height;
        p.lines.retain(|l| match zone(h, l.base) {
            Some((top, b)) => count(&norm_key(&l.text), top, b) < need,
            None => true,
        });
    }
}

fn right_margin(lines: &[Line]) -> f32 {
    let mut xs: Vec<f32> = lines.iter().filter(|l| l.text.chars().count() >= 40).map(|l| l.x1).collect();
    if xs.is_empty() {
        return lines.iter().map(|l| l.x1).fold(0.0, f32::max);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs[(xs.len() * 85 / 100).min(xs.len() - 1)]
}

fn sorted_lines(p: &Page) -> Vec<&Line> {
    let mut lines: Vec<&Line> = p.lines.iter().collect();
    lines.sort_by(|a, b| {
        if a.col != b.col {
            return a.col.cmp(&b.col);
        }
        let tol = 0.35 * a.size.max(b.size);
        if (a.base - b.base).abs() <= tol { a.x0.partial_cmp(&b.x0).unwrap() } else { b.base.partial_cmp(&a.base).unwrap() }
    });
    lines
}

/// Gap (in font sizes) that separates paragraphs. Double-spaced documents have
/// a normal line gap of ~2, so the threshold is learned from wrapped lines:
/// a full-width line followed by a same-style line must be a continuation.
fn paragraph_gap(pages: &[&Page]) -> f32 {
    let mut samples: Vec<f32> = Vec::new();
    for p in pages {
        let right = right_margin(&p.lines);
        for w in sorted_lines(p).windows(2) {
            let (a, b) = (w[0], w[1]);
            let gap = (a.base - b.base) / b.size.max(a.size);
            if (a.size - b.size).abs() <= 0.6
                && a.bold == b.bold
                && !a.tabular
                && !b.tabular
                && a.x1 >= right - 3.0 * a.size
                && gap > 0.5
                && gap < 3.2
                && (a.x0 - b.x0).abs() < 1.5 * a.size
            {
                samples.push(gap);
            }
        }
    }
    if samples.len() < 5 {
        return 1.6;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (1.3 * samples[samples.len() / 2]).max(1.55)
}

fn blocks_for_page(p: &Page, para_gap: f32, vocab: &Vocab) -> Vec<Blk> {
    let lines = sorted_lines(p);
    let right = right_margin(&p.lines);
    let mut out: Vec<Blk> = Vec::new();
    let mut prev: Option<&Line> = None;
    for l in lines {
        let marker = list_marker(&l.text);
        let mut hard = false;
        let mut brk = true;
        if let (Some(pl), Some(b)) = (prev, out.last()) {
            let is_item = matches!(b.kind, Kind::Item(_));
            let gap = pl.base - l.base;
            let style = (pl.size - l.size).abs() <= 0.6 && pl.bold == l.bold;
            let hyph = pl.text.ends_with('-');
            let short = !hyph && pl.x1 < right - 10.0 * pl.size;
            let indent = !is_item && l.x0 - pl.x0 > 0.8 * l.size && !pl.tabular && !l.tabular;
            brk = !style || gap > para_gap * l.size.max(pl.size) || marker.is_some() || indent || gap <= 0.0 || pl.col != l.col;
            hard = pl.tabular || l.tabular || short;
        }
        let text_after = |m: Option<(Item, usize)>| match m {
            Some((Item::Bullet, n)) => l.text[n..].to_string(),
            _ => l.text.clone(),
        };
        if brk {
            let kind = match marker {
                Some((it, _)) => Kind::Item(it),
                None => Kind::Para,
            };
            out.push(Blk { kind, logical: vec![text_after(marker)], size: l.size, bold: l.bold, ends_open: false, tabular: l.tabular });
        } else {
            let b = out.last_mut().unwrap();
            b.tabular |= l.tabular;
            if hard {
                b.logical.push(l.text.clone());
            } else {
                let last = b.logical.last_mut().unwrap();
                join_wrapped(last, &l.text, vocab);
            }
        }
        prev = Some(l);
    }
    for b in &mut out {
        let t = b.logical.last().map(String::as_str).unwrap_or("");
        b.ends_open = ends_open(t);
    }
    out
}

fn looks_like_title(t: &str) -> bool {
    let words: Vec<&str> = t.split_whitespace().collect();
    let n = t.chars().count();
    if words.is_empty() || words.len() > 8 || n > 60 || n < 3 || t.contains('\u{2003}') || t.contains('…') {
        return false;
    }
    if t.ends_with(['.', ',', ';', ':', '?', '!']) || t.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    // a middle initial marks a person's name, not a section title
    if words.iter().any(|w| w.len() == 2 && w.ends_with('.') && w.chars().next().is_some_and(|c| c.is_uppercase())) {
        return false;
    }
    const SMALL: [&str; 14] = ["and", "of", "the", "for", "to", "in", "on", "a", "an", "or", "with", "by", "at", "from"];
    let first_upper = t.chars().next().is_some_and(|c| c.is_uppercase());
    first_upper && words.iter().all(|w| SMALL.contains(&w.to_lowercase().as_str()) || w.chars().next().is_some_and(|c| c.is_uppercase() || !c.is_alphabetic()))
}

/// A short title-case line standing alone above a real paragraph is a section
/// title even in the body font. Pages that would get many (slides, contents
/// pages) get none.
fn promote_section_titles(bl: &mut Vec<Blk>, body: f32) {
    // A title directly above its paragraph was joined into it as a short first line: split it off.
    let mut i = 0;
    let mut hits: Vec<usize> = Vec::new();
    while i < bl.len() {
        let styled = bl[i].kind == Kind::Para && bl[i].size >= 0.9 * body && bl[i].size < 1.2 * body;
        if styled && bl[i].logical.len() >= 2 && looks_like_title(&bl[i].logical[0]) && bl[i].logical[1..].iter().map(|l| l.chars().count()).sum::<usize>() >= 80 {
            let mut rest = bl[i].clone();
            rest.logical.remove(0);
            bl[i].logical.truncate(1);
            bl.insert(i + 1, rest);
        }
        if styled && bl[i].logical.len() == 1 && looks_like_title(&bl[i].logical[0]) {
            let next = bl.get(i + 1);
            if next.is_some_and(|n| n.kind == Kind::Para && n.logical.iter().map(|l| l.chars().count()).sum::<usize>() >= 80) {
                hits.push(i);
            }
        }
        i += 1;
    }
    if hits.len() <= 6 {
        for i in hits {
            bl[i].kind = Kind::Heading;
        }
    }
}

fn classify_heading(b: &mut Blk, body: f32) -> bool {
    if b.kind != Kind::Para && !(matches!(b.kind, Kind::Item(_)) && b.bold) {
        return false;
    }
    let text = b.logical.join(" ");
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    if text.contains('…') || b.logical.len() > 3 || chars == 0 || chars > 120 || words > 16 {
        return false;
    }
    if text.ends_with('.') && words > 8 {
        return false;
    }
    let big = b.size >= 1.2 * body;
    let boldhead = b.bold && b.size >= 0.95 * body && chars <= 100 && words <= 12 && !text.ends_with(',');
    if !(big || boldhead) {
        return false;
    }
    if let Kind::Item(_) = b.kind {
        // a bold "1. Introduction": keep the number in the heading
    }
    true
}

/// Delimits a page marker while the text is assembled. Marker text must not go
/// through inline escaping, so it is written last.
/// U+FFFF is dropped from every input, so it cannot occur in the text itself.
const MARK: char = '\u{FFFF}';

fn mark(n: usize) -> String {
    format!("{MARK}{n}{MARK}")
}

fn strip_marks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        if c == MARK {
            inside = !inside;
        } else if !inside {
            out.push(c);
        }
    }
    out
}

fn expand_marks(s: &str, file: &str) -> String {
    let mut out = String::with_capacity(s.len() + 64);
    let mut num = String::new();
    let mut inside = false;
    for c in s.chars() {
        match (c == MARK, inside) {
            (true, false) => inside = true,
            (true, true) => {
                out.push_str(&format!("[p. {num}]({}#page={num})", url_segment(file)));
                num.clear();
                inside = false;
            }
            (false, true) => num.push(c),
            (false, false) => out.push(c),
        }
    }
    out
}

/// Percent-encode a file name as one URL path segment: ASCII letters, digits
/// and `-._~` stay, every other byte of its UTF-8 becomes `%XX`.
fn url_segment(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn leading_number(s: &str) -> Option<u32> {
    let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// Rendered parts in order. Consecutive list items are tight; across a page
/// break a numbered list stays one list only if the numbers carry on.
#[derive(Default)]
struct Emit {
    parts: Vec<(String, bool)>,
    /// Last part was a list item: its number, if numbered.
    prev_item: Option<Option<u32>>,
}

impl Emit {
    fn block(&mut self, b: &Blk, lvl: usize, page_start: bool) {
        let numbered = b.kind == Kind::Item(Item::Numbered);
        let item = numbered || b.kind == Kind::Item(Item::Bullet);
        let number = if numbered { leading_number(&b.logical[0]) } else { None };
        let tight = item
            && self.prev_item.is_some_and(|p| match (page_start, p, number) {
                (false, ..) => true,
                (true, Some(a), Some(n)) => n == a + 1,
                (true, None, None) => true,
                _ => false,
            });
        self.parts.push((render_block(b, lvl), tight));
        self.prev_item = item.then_some(number);
    }

    fn paragraph(&mut self, s: String) {
        self.parts.push((s, false));
        self.prev_item = None;
    }
}

/// Append `first` (the start of the next page) to the open block `c`, with the
/// page marker at the join. A word hyphenated across the break is repaired and
/// the marker goes after it.
fn join_across(c: &mut Blk, first: &str, page: Option<usize>, vocab: &Vocab) {
    let last = c.logical.last_mut().unwrap();
    let Some(n) = page else {
        return join_wrapped(last, first, vocab);
    };
    let cut_word = last.strip_suffix('-').and_then(|s| s.chars().last()).is_some_and(|ch| ch.is_alphabetic());
    if cut_word {
        let (head, tail) = first.split_once(' ').unwrap_or((first, ""));
        join_wrapped(last, head, vocab);
        last.push(' ');
        last.push_str(&mark(n));
        if !tail.is_empty() {
            last.push(' ');
            last.push_str(tail);
        }
    } else {
        last.push(' ');
        last.push_str(&mark(n));
        last.push(' ');
        last.push_str(first);
    }
}

/// Put the marker first in a paragraph or list item (after the number of a numbered one).
fn prefix_marker(b: &mut Blk, n: usize) {
    let at = if b.kind == Kind::Item(Item::Numbered) { list_marker(&b.logical[0]).map_or(0, |m| m.1) } else { 0 };
    b.logical[0].insert_str(at, &format!("{} ", mark(n)));
}

const NOTE_LEAD_ALL: &str = "This text was read by machine from a scan and can contain errors.";
const NOTE_LEAD_SOME: &str = "Parts of this text were read by machine from scanned pages and can contain errors.";

pub fn render(contents: Vec<Content>, meta_title: Option<String>, file_name: &str) -> Rendered {
    // 1. running headers/footers over the text pages
    let mut contents = contents;
    {
        let mut refs: Vec<&mut Page> = contents
            .iter_mut()
            .filter_map(|c| if let Content::Lines(p) = c { Some(p) } else { None })
            .collect();
        strip_running(&mut refs);
    }
    {
        let mut refs: Vec<&mut String> = contents.iter_mut().filter_map(|c| if let Content::Ocr(t) = c { Some(t) } else { None }).collect();
        strip_ocr_running(&mut refs);
    }
    let mut vocab = Vocab::new();
    for c in &contents {
        match c {
            Content::Lines(p) => p.lines.iter().for_each(|l| add_vocab(&mut vocab, &l.text)),
            Content::Ocr(t) => add_vocab(&mut vocab, t),
        }
    }
    let all: Vec<&Page> = contents.iter().filter_map(|c| if let Content::Lines(p) = c { Some(p) } else { None }).collect();
    let body = mode_size(&all);
    let para_gap = paragraph_gap(&all);

    // 2. blocks per page, headings classified
    let mut page_blocks: Vec<Vec<Blk>> = Vec::new();
    let mut is_ocr: Vec<bool> = Vec::new();
    for c in &contents {
        match c {
            Content::Lines(p) => {
                let mut bl = blocks_for_page(p, para_gap, &vocab);
                for b in bl.iter_mut() {
                    if classify_heading(b, body) {
                        b.kind = Kind::Heading;
                    }
                }
                promote_section_titles(&mut bl, body);
                page_blocks.push(bl);
            }
            Content::Ocr(t) => page_blocks.push(ocr_blocks(t, &vocab)),
        }
        is_ocr.push(matches!(c, Content::Ocr(_)));
    }

    // 3. title from the first text page
    let mut title: Option<String> = None;
    let mut drop_title: Option<(usize, usize)> = None;
    if let Some((pi, bl)) = page_blocks.iter().enumerate().find(|(i, b)| !is_ocr[*i] && !b.is_empty()) {
        let mut cands: Vec<(usize, &Blk)> = bl.iter().enumerate().filter(|(_, b)| b.kind == Kind::Heading && b.size >= 1.2 * body).collect();
        cands.sort_by(|(ia, a), (ib, b)| b.size.partial_cmp(&a.size).unwrap().then(ia.cmp(ib)));
        for (bi, b) in cands {
            let t = b.logical.join(" ");
            if t.chars().count() <= 120 && !is_generic_title(&t) {
                title = Some(t);
                if bi == 0 {
                    drop_title = Some((pi, bi));
                }
                break;
            }
        }
    }
    if title.is_none() {
        title = meta_title.and_then(|t| meaningful_metadata_title(&t));
    }

    // 4. heading levels by size rank (<= 3)
    let key = |b: &Blk| -> i32 { if b.size >= 1.2 * body { b.size.round() as i32 } else { 0 } };
    let mut keys: Vec<i32> = page_blocks.iter().flatten().filter(|b| b.kind == Kind::Heading).map(key).collect();
    keys.sort_unstable_by(|a, b| b.cmp(a));
    keys.dedup();
    let level = |b: &Blk| -> usize { keys.iter().position(|k| *k == key(b)).unwrap_or(0).min(2) };

    // 5. flatten with page markers; a paragraph or item left open at the end of
    // a page continues into a lowercase paragraph at the top of the next
        let mut e = Emit::default();
    let mut carry: Option<(Blk, bool)> = None; // open block of the previous page, was it first on its page
    for (pi, bl) in page_blocks.iter().enumerate() {
        let marker = (pi > 0).then_some(pi + 1);
        let blocks: Vec<&Blk> = bl.iter().enumerate().filter(|(bi, _)| drop_title != Some((pi, *bi))).map(|(_, b)| b).collect();
        if blocks.is_empty() {
            if let Some((c, start)) = carry.take() {
                e.block(&c, 0, start);
            }
            if let Some(n) = marker {
                e.paragraph(mark(n));
            }
            continue;
        }
        for (bi, b) in blocks.iter().enumerate() {
            let mut b = (*b).clone();
            let mut page_start = bi == 0;
            if bi == 0 {
                let mut marked = false;
                if let Some((mut c, c_start)) = carry.take() {
                    if b.kind == Kind::Para && !b.tabular && b.logical[0].chars().next().is_some_and(|ch| ch.is_lowercase()) {
                        let first = b.logical.remove(0);
                        join_across(&mut c, &first, marker, &vocab);
                        c.logical.append(&mut b.logical);
                        c.ends_open = b.ends_open;
                        b = c;
                        page_start = c_start;
                        marked = true;
                    } else {
                        e.block(&c, 0, c_start);
                    }
                }
                if let (Some(n), false) = (marker, marked) {
                    if matches!(b.kind, Kind::Para | Kind::Item(_)) && !b.tabular {
                        prefix_marker(&mut b, n);
                    } else {
                        e.paragraph(mark(n));
                    }
                }
            }
            if bi + 1 == blocks.len() && matches!(b.kind, Kind::Para | Kind::Item(_)) && !b.tabular && b.ends_open {
                carry = Some((b, page_start));
                continue;
            }
            let lvl = if b.kind == Kind::Heading { level(&b) } else { 0 };
            e.block(&b, lvl, page_start);
        }
    }
    if let Some((c, start)) = carry.take() {
        e.block(&c, 0, start);
    }

    let mut md = String::new();
    for (i, (s, tight)) in e.parts.iter().enumerate() {
        if i > 0 {
            md.push_str(if *tight { "\n" } else { "\n\n" });
        }
        md.push_str(s);
    }
    let body_chars = strip_marks(&md).chars().filter(|c| !c.is_whitespace()).count();

    // 6. the note and the description. A blank page that went through OCR is
    // neutral: it says nothing about whether the document is a scan.
    let pages = page_blocks.len();
    let ocr_pages = is_ocr.iter().filter(|o| **o).count();
    let ocr_read = page_blocks.iter().zip(&is_ocr).filter(|(b, o)| **o && !b.is_empty()).count();
    let mut description = if pages == 1 { "1 page".to_string() } else { format!("{pages} pages") };
    if ocr_read > 0 {
        let all_ocr = ocr_pages == pages;
        description.push_str(if all_ocr { ", scanned" } else { ", partly scanned" });
        let lead = if all_ocr { NOTE_LEAD_ALL } else { NOTE_LEAD_SOME };
        let record = match pages {
            1 => "The PDF is the document of record.",
            _ => "The PDF is the document of record; a page number opens it at that page.",
        };
        md = format!("> [!note]\n> {lead} {record}\n\n{md}");
    }
    Rendered { markdown: expand_marks(&md, file_name), title, description, body_chars }
}

fn render_block(b: &Blk, lvl: usize) -> String {
    match &b.kind {
        Kind::Heading => {
            let t = escape_line_start(&escape_inline(&b.logical.join(" ")));
            format!("{} {}", "#".repeat(lvl + 2), t)
        }
        Kind::Item(Item::Bullet) => item_text("- ", b),
        Kind::Item(Item::Numbered) => item_text("\u{0}", b),
        Kind::Item(Item::Other) | Kind::Para => item_text("", b),
    }
}

fn item_text(prefix: &str, b: &Blk) -> String {
    let mut s = String::new();
    for (i, l) in b.logical.iter().enumerate() {
        if i > 0 {
            s.push_str("  \n");
            if prefix == "- " {
                s.push_str("  ");
            }
        }
        let e = escape_inline(l);
        s.push_str(&if i == 0 && !prefix.is_empty() { e } else { escape_line_start(&e) });
    }
    format!("{}{s}", prefix.trim_matches('\u{0}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ocr(pages: &[&str]) -> String {
        render(pages.iter().map(|p| Content::Ocr(p.to_string())).collect(), None, "x.pdf").markdown
    }

    fn note_free(md: &str) -> &str {
        md.split_once("\n\n").unwrap().1
    }

    #[test]
    fn open_item_continues_and_the_list_goes_on() {
        let md = ocr(&["Opening line.\n\n1. Alpha item.\n2. Beta item continues with the", "next words here.\n3. Gamma item."]);
        assert_eq!(note_free(&md), "Opening line.\n\n1. Alpha item.\n2. Beta item continues with the [p. 2](x.pdf#page=2) next words here.\n3. Gamma item.");
    }

    #[test]
    fn numbered_list_resuming_on_a_page_is_tight_only_if_numbers_follow() {
        let md = ocr(&["Opening line.\n\n1. Alpha item.\n2. Beta item.", "3. Gamma item.\n\nClosing line."]);
        assert!(note_free(&md).contains("2. Beta item.\n3. [p. 2](x.pdf#page=2) Gamma item.\n\nClosing"), "{md}");
        let md = ocr(&["Opening line.\n\n1. Alpha item.\n2. Beta item.", "1. Restarted item.\n\nClosing line."]);
        assert!(note_free(&md).contains("2. Beta item.\n\n1. [p. 2](x.pdf#page=2) Restarted"), "{md}");
    }

    #[test]
    fn only_a_lowercase_paragraph_continues_an_open_block() {
        for next in ["Upper start of a page.", "2019 was a year.", "- bullet.", "a. lettered.", "1. numbered."] {
            let md = ocr(&["A paragraph with no end", next]);
            assert!(note_free(&md).starts_with("A paragraph with no end\n\n"), "{next}: {md}");
        }
        assert!(note_free(&ocr(&["A paragraph with no end", "lower start."])).starts_with("A paragraph with no end [p. 2]"));
    }

    #[test]
    fn closing_quotes_and_brackets_after_final_punctuation_close_a_block() {
        for closed in ["He said \"stop.\"", "It ended (finally.)", "Is it so?”", "Note:"] {
            assert!(!ends_open(closed), "{closed}");
        }
        for open in ["It ended (see page 3)", "and then", "x,"] {
            assert!(ends_open(open), "{open}");
        }
    }

    #[test]
    fn file_names_are_encoded_as_one_url_segment() {
        assert_eq!(url_segment("report 2019.pdf"), "report%202019.pdf");
        assert_eq!(url_segment("a(b)#c%d~e-f_g.pdf"), "a%28b%29%23c%25d~e-f_g.pdf");
        assert_eq!(url_segment("é.pdf"), "%C3%A9.pdf");
    }

    #[test]
    fn page_numbers() {
        assert!(is_page_number("Page 3 of 9"));
        assert!(is_page_number("- 12 -"));
        assert!(is_page_number("4"));
        assert!(!is_page_number("Corrective Action #4"));
    }

    #[test]
    fn dehyphenate() {
        let mut a = "the respon-".to_string();
        join_wrapped(&mut a, "sibility for", &Vocab::new());
        assert_eq!(a, "the responsibility for");
        let mut a = "Non-".to_string();
        join_wrapped(&mut a, "Profit", &Vocab::new());
        assert_eq!(a, "Non-Profit");
        let v: Vocab = ["federal", "level", "federallevel"].iter().take(2).map(|s| s.to_string()).collect();
        let mut a = "the federal-".to_string();
        join_wrapped(&mut a, "level rules", &v);
        assert_eq!(a, "the federal-level rules");
    }

    #[test]
    fn markers() {
        assert_eq!(list_marker("1. Foo").map(|m| m.0), Some(Item::Numbered));
        assert_eq!(list_marker("• Foo").map(|m| m.0), Some(Item::Bullet));
        assert_eq!(list_marker("a. Foo").map(|m| m.0), Some(Item::Other));
        assert!(list_marker("2019 was a year").is_none());
        assert!(list_marker("J. Smith").is_none());
    }

    #[test]
    fn escapes() {
        assert_eq!(escape_line_start(&escape_inline("# not heading")), "\\# not heading");
        assert_eq!(escape_inline("see [[x]] and *this*"), "see \\[\\[x]] and \\*this\\*");
        assert_eq!(escape_line_start("2019. Year"), "2019\\. Year");
    }

    #[test]
    fn double_spaced_text_stays_one_paragraph() {
        let mk = |i: usize, w: &str| Line { x0: 72.0, x1: 540.0, base: 700.0 - 24.0 * i as f32, size: 12.0, bold: false, text: w.to_string(), tabular: false, col: 0 };
        let mut lines: Vec<Line> = (0..6).map(|i| mk(i, "word word word word word word word word word word word word")).collect();
        lines.push(Line { x1: 300.0, ..mk(6, "last line of paragraph one.") });
        lines.extend((8..12).map(|i| mk(i, "second paragraph words words words words words words words")));
        let r = render(vec![Content::Lines(Page { height: 792.0, lines })], None, "x.pdf");
        assert_eq!(r.markdown.split("\n\n").count(), 2, "{}", r.markdown);
    }
}
