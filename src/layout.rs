//! Turn positioned text lines into markdown. Pure functions, no pdfium here.

use crate::front;
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
    /// Reading-order column group on the page (0 for single-column pages).
    pub col: u8,
}

pub struct Page {
    pub height: f32,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone)]
struct Blk {
    logical: Vec<String>,
    size: f32,
    ends_open: bool,
}

pub struct Rendered {
    pub markdown: String,
    pub title: Option<String>,
    /// The first full date on the first page, `YYYY-MM-DD`.
    pub date: Option<String>,
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

/// Backslash-escape what would read as inline markdown, so the text renders as itself.
pub fn escape_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        let escape = match c {
            '\\' | '*' | '`' | '[' | ']' | '|' | '~' => true,
            '_' => !(prev.is_some_and(|p| p.is_alphanumeric()) && next.is_some_and(|n| n.is_alphanumeric())),
            '<' => next.is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!' || n == '?'),
            '&' => next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#'),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape what matters at the start of a line (after inline escaping).
fn escape_line_start(s: &str) -> String {
    let t = s.trim_start();
    let first = t.chars().next();
    match first {
        Some('#') | Some('>') | Some('+') | Some('-') | Some('=') => format!("\\{t}"),
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
        let mut text = String::new();
        for l in para.lines().map(str::trim).filter(|l| !l.is_empty()) {
            join_wrapped(&mut text, l, vocab);
        }
        if text.is_empty() {
            continue;
        }
        let open = ends_open(&text);
        // Tesseract sets a short last line apart as its own paragraph.
        let widow = text.chars().next().is_some_and(|c| c.is_lowercase());
        match blocks.last_mut() {
            Some(prev) if widow && prev.ends_open => {
                join_wrapped(prev.logical.last_mut().unwrap(), &text, vocab);
                prev.ends_open = open;
            }
            _ => blocks.push(Blk { logical: vec![text], size: 0.0, ends_open: open }),
        }
    }
    blocks
}

/// A block whose last line lacks sentence-final punctuation (optionally
/// followed by closing quotes or brackets) may continue on the next page. A
/// semicolon or a colon does not end a sentence: "…protected groups;" at the
/// foot of a page is followed by "participate in career fairs…" on the next.
fn ends_open(t: &str) -> bool {
    !t.trim_end_matches(['"', '\'', '”', '’', ')', ']', '»']).ends_with(['.', '?', '!'])
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

/// Titles that authoring tools write when the author did not.
const DEFAULT_TITLES: [&str; 14] = [
    "teammate report", "powerpoint presentation", "untitled", "title", "table of contents", "title of presentation",
    "presentation", "document", "slide 1", "microsoft word", "microsoft powerpoint", "pdf", "report", "scan",
];

fn all_digits_after(l: &str, prefix: &str) -> bool {
    l.strip_prefix(prefix).is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
}

/// A metadata title worth showing, or None when it is a tool default or machine-made.
pub fn meaningful_metadata_title(t: &str, file_name: &str) -> Option<String> {
    let t = t.trim();
    let t = t.strip_prefix("Microsoft Word - ").or_else(|| t.strip_prefix("Microsoft PowerPoint - ")).unwrap_or(t).trim();
    let l = t.to_lowercase();
    let stem = std::path::Path::new(file_name).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let numbered = |p: &str| l.strip_prefix(p).is_some_and(|r| !r.trim().is_empty() && r.trim().chars().all(|c| c.is_ascii_digit()));
    let junk = DEFAULT_TITLES.contains(&l.as_str())
        || [".pdf", ".doc", ".docx", ".ppt", ".pptx"].iter().any(|e| l.ends_with(e))
        || numbered("document")
        || numbered("untitled")
        || numbered("xml")
        || numbered("scan")
        || l.starts_with("km_c")
        || all_digits_after(&l, "scan")
        || (!l.is_empty() && (l == stem || l == file_name.to_lowercase()))
        || !t.contains(char::is_whitespace);
    let n = t.chars().count();
    if junk || !(3..=150).contains(&n) || !t.chars().any(|c| c.is_alphabetic()) || t.contains('\u{FFFD}') {
        return None;
    }
    Some(t.to_string())
}

/// "521_BERS_2020_SHPRA.pdf" becomes "521 BERS 2020 SHPRA".
fn title_from_file_name(file_name: &str) -> Option<String> {
    let stem = std::path::Path::new(file_name).file_stem()?.to_string_lossy().into_owned();
    let t = stem.replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
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
        let mut hard = false;
        let mut brk = true;
        if let Some(pl) = prev {
            let gap = pl.base - l.base;
            let style = (pl.size - l.size).abs() <= 0.6 && pl.bold == l.bold;
            let hyph = pl.text.ends_with('-');
            let short = !hyph && pl.x1 < right - 10.0 * pl.size;
            let indent = l.x0 - pl.x0 > 0.8 * l.size;
            brk = !style || gap > para_gap * l.size.max(pl.size) || indent || gap <= 0.0 || pl.col != l.col;
            hard = short;
        }
        if brk {
            out.push(Blk { logical: vec![l.text.clone()], size: l.size, ends_open: false });
        } else {
            let b = out.last_mut().unwrap();
            if hard {
                b.logical.push(l.text.clone());
            } else {
                join_wrapped(b.logical.last_mut().unwrap(), &l.text, vocab);
            }
        }
        prev = Some(l);
    }
    for b in &mut out {
        b.ends_open = ends_open(b.logical.last().map(String::as_str).unwrap_or(""));
    }
    out
}

/// A short block in a type size well above the body text may be the document's title.
fn title_candidate(b: &Blk) -> bool {
    let text = b.logical.join(" ");
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    !(text.contains('…') || b.logical.len() > 3 || chars == 0 || chars > 120 || words > 16 || (text.ends_with('.') && words > 8))
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

/// Put the marker first in a paragraph.
fn prefix_marker(b: &mut Blk, n: usize) {
    b.logical[0].insert_str(0, &format!("{} ", mark(n)));
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

    // 2. blocks per page
    let mut page_blocks: Vec<Vec<Blk>> = Vec::new();
    let mut is_ocr: Vec<bool> = Vec::new();
    for c in &contents {
        match c {
            Content::Lines(p) => page_blocks.push(blocks_for_page(p, para_gap, &vocab)),
            Content::Ocr(t) => page_blocks.push(ocr_blocks(t, &vocab)),
        }
        is_ocr.push(matches!(c, Content::Ocr(_)));
    }

    // 3. title: the first page's subject line, else its largest type, else metadata, else the file name
    let first_page: Vec<&[String]> = page_blocks
        .iter()
        .find(|b| !b.is_empty())
        .map(|b| b.iter().map(|b| b.logical.as_slice()).collect())
        .unwrap_or_default();
    let date = front::first_date(&first_page);
    let mut title: Option<String> = front::subject_title(&first_page);
    let mut drop_title: Option<(usize, usize)> = None;
    if let (None, Some((pi, bl))) = (&title, page_blocks.iter().enumerate().find(|(i, b)| !is_ocr[*i] && !b.is_empty())) {
        let mut cands: Vec<(usize, &Blk)> = bl.iter().enumerate().filter(|(_, b)| b.size >= 1.2 * body && title_candidate(b)).collect();
        cands.sort_by(|(ia, a), (ib, b)| b.size.partial_cmp(&a.size).unwrap().then(ia.cmp(ib)));
        for (bi, b) in cands {
            let t = b.logical.join(" ");
            if t.chars().count() <= 120 && !is_generic_title(&t) && !front::is_letterhead(&t) {
                title = Some(t);
                if bi == 0 {
                    drop_title = Some((pi, bi));
                }
                break;
            }
        }
    }
    if title.is_none() {
        title = meta_title.and_then(|t| meaningful_metadata_title(&t, file_name));
    }
    if title.is_none() {
        title = title_from_file_name(file_name);
    }

    // 4. flatten with page markers; a paragraph left open at the end of a page
    // continues into a lowercase paragraph at the top of the next
    let mut parts: Vec<String> = Vec::new();
    let mut carry: Option<Blk> = None;
    for (pi, bl) in page_blocks.iter().enumerate() {
        let marker = (pi > 0).then_some(pi + 1);
        let blocks: Vec<&Blk> = bl.iter().enumerate().filter(|(bi, _)| drop_title != Some((pi, *bi))).map(|(_, b)| b).collect();
        if blocks.is_empty() {
            if let Some(c) = carry.take() {
                parts.push(render_block(&c));
            }
            if let Some(n) = marker {
                parts.push(mark(n));
            }
            continue;
        }
        for (bi, b) in blocks.iter().enumerate() {
            let mut b = (*b).clone();
            if bi == 0 {
                let mut marked = false;
                if let Some(mut c) = carry.take() {
                    if b.logical[0].chars().next().is_some_and(|ch| ch.is_lowercase()) {
                        let first = b.logical.remove(0);
                        join_across(&mut c, &first, marker, &vocab);
                        c.logical.append(&mut b.logical);
                        c.ends_open = b.ends_open;
                        b = c;
                        marked = true;
                    } else {
                        parts.push(render_block(&c));
                    }
                }
                if let (Some(n), false) = (marker, marked) {
                    prefix_marker(&mut b, n);
                }
            }
            if bi + 1 == blocks.len() && b.ends_open {
                carry = Some(b);
                continue;
            }
            parts.push(render_block(&b));
        }
    }
    if let Some(c) = carry.take() {
        parts.push(render_block(&c));
    }

    let mut md = parts.join("\n\n");
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
    Rendered { markdown: expand_marks(&md, file_name), title, date, description, body_chars }
}

/// Extracted text is literal: escape it so no part of it reads as markdown. The
/// page markers are not yet in it (see `mark`), so they stay intact.
fn render_block(b: &Blk) -> String {
    b.logical.iter().map(|l| escape_line_start(&escape_inline(l))).collect::<Vec<_>>().join("  \n")
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
    fn ocr_widow_line_rejoins_its_paragraph() {
        let md = ocr(&["First paragraph that runs on and on and\n\ncandidates.\n\nNew paragraph."]);
        assert_eq!(note_free(&md), "First paragraph that runs on and on and candidates.\n\nNew paragraph.");
    }

    #[test]
    fn only_a_lowercase_paragraph_continues_an_open_block() {
        for next in ["Upper start of a page.", "2019 was a year.", "- bullet.", "1. numbered."] {
            let md = ocr(&["A paragraph with no end", next]);
            assert!(note_free(&md).starts_with("A paragraph with no end\n\n"), "{next}: {md}");
        }
        assert!(note_free(&ocr(&["A paragraph with no end", "lower start."])).starts_with("A paragraph with no end [p. 2]"));
    }

    #[test]
    fn closing_quotes_and_brackets_after_final_punctuation_close_a_block() {
        for closed in ["He said \"stop.\"", "It ended (finally.)", "Is it so?”"] {
            assert!(!ends_open(closed), "{closed}");
        }
        for open in ["It ended (see page 3)", "and then", "x,", "protected groups;", "Note:"] {
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

    /// The HTML `md` renders to, asserted free of any markup element.
    fn html_of(md: &str) -> String {
        let mut h = String::new();
        pulldown_cmark::html::push_html(&mut h, pulldown_cmark::Parser::new_ext(md, pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH));
        for tag in ["<ul", "<ol", "<li", "<h1", "<h2", "<h3", "<blockquote", "<em", "<strong", "<a ", "<table", "<code", "<pre", "<hr", "<del", "<script", "<b>", "<div"] {
            assert!(!h.contains(tag), "{tag} in {h} for {md}");
        }
        h
    }

    /// `text` as one escaped paragraph renders as exactly itself.
    fn assert_literal(text: &str) {
        let md = render_block(&Blk { logical: vec![text.to_string()], size: 0.0, ends_open: false });
        let html = html_of(&md);
        let plain = html.trim().strip_prefix("<p>").and_then(|h| h.strip_suffix("</p>")).expect(&html);
        let plain = plain.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
        assert_eq!(plain, text, "escaped as {md:?}");
    }

    #[test]
    fn text_that_looks_like_markdown_renders_literally() {
        let leader = format!("{}M A L E {}", "- ".repeat(65), "- ".repeat(65));
        for t in [
            leader.trim_end(),
            "- - - - M A L E - - - -",
            "# Not a heading",
            "> not a quote",
            "1. not a list",
            "7) not a list",
            "*not* _emphasis_ and **strong**",
            "a | b | c",
            "see [here](http://x.test) and ![img](y.png) and [[wiki]]",
            "<b>raw</b> <script>x</script> <!-- c -->",
            "`code` and ``more``",
            "snake_case_word stays",
            "tilde ~~struck~~ text",
            "R&D &amp; &#35; done",
            "back\\slash \\* star",
            "+ plus",
            "~~~",
            "---",
            "===",
            "    four spaces",
        ] {
            assert_literal(t.trim_start());
        }
    }

    #[test]
    fn leader_rule_is_escaped_not_nested() {
        assert_eq!(note_free(&ocr(&["- - - - M A L E - - - -"])), "\\- - - - M A L E - - - -");
        let md = ocr(&["Title line\n---"]);
        assert_eq!(note_free(&md), "Title line ---");
        let md = render_block(&Blk { logical: vec!["Title line".into(), "---".into(), "===".into()], size: 0.0, ends_open: false });
        assert_eq!(md, "Title line  \n\\---  \n\\===");
        html_of(&md);
    }

    #[test]
    fn own_markup_is_not_escaped() {
        let md = ocr(&["Plain *text* with no end", "continues here.\n\nNext one."]);
        assert!(md.starts_with("> [!note]\n> This text was read by machine"), "{md}");
        assert!(md.contains("Plain \\*text\\* with no end [p. 2](x.pdf#page=2) continues here.\n\nNext one."), "{md}");
    }

    #[test]
    fn double_spaced_text_stays_one_paragraph() {
        let mk = |i: usize, w: &str| Line { x0: 72.0, x1: 540.0, base: 700.0 - 24.0 * i as f32, size: 12.0, bold: false, text: w.to_string(), col: 0 };
        let mut lines: Vec<Line> = (0..6).map(|i| mk(i, "word word word word word word word word word word word word")).collect();
        lines.push(Line { x1: 300.0, ..mk(6, "last line of paragraph one.") });
        lines.extend((8..12).map(|i| mk(i, "second paragraph words words words words words words words")));
        let r = render(vec![Content::Lines(Page { height: 792.0, lines })], None, "x.pdf");
        assert_eq!(r.markdown.split("\n\n").count(), 2, "{}", r.markdown);
    }

    #[test]
    fn tool_defaults_and_machine_names_are_not_titles() {
        for junk in [
            "TeamMate Report", "PowerPoint Presentation", "Untitled", "untitled 3", "Title", "Table of Contents", "TITLE OF PRESENTATION",
            "Document1", "Microsoft Word - Document1", "KM_C368-20220127131051", "Scan0042", "XML 4", "Single", "a_b.pdf", "x.docx",
        ] {
            assert_eq!(meaningful_metadata_title(junk, "x.pdf"), None, "{junk}");
        }
        assert_eq!(meaningful_metadata_title("Budget Notes", "Budget Notes.pdf"), None);
        assert_eq!(meaningful_metadata_title("budget-2020", "budget-2020.pdf"), None);
    }

    #[test]
    fn real_metadata_titles_are_kept() {
        for ok in ["Equal Employment Practices Commission 2022 Annual Report", "NYC EEPC presentation Outline"] {
            assert_eq!(meaningful_metadata_title(ok, "x.pdf").as_deref(), Some(ok));
        }
        assert_eq!(meaningful_metadata_title("Microsoft Word - Annual Plan 2020", "x.pdf").as_deref(), Some("Annual Plan 2020"));
    }

    #[test]
    fn title_falls_back_from_metadata_to_the_file_name() {
        let r = |meta: Option<&str>| render(vec![Content::Ocr("Some scanned words here for the page.".into())], meta.map(String::from), "521_BERS_2020_SHPRA.pdf").title;
        assert_eq!(r(Some("Real Metadata Title")).as_deref(), Some("Real Metadata Title"));
        assert_eq!(r(Some("TeamMate Report")).as_deref(), Some("521 BERS 2020 SHPRA"));
        assert_eq!(r(None).as_deref(), Some("521 BERS 2020 SHPRA"));
    }

    #[test]
    fn a_letters_subject_line_is_its_title_and_its_date_is_a_fact() {
        let letter = "BY EMAIL\n\nJune 9, 2020\n\nDear Chair:\n\nAudit Status: Evaluation of Sexual Harassment Prevention and Response Practices for Audit Period January 1, 2018 to December 31, 2019. Determination: PRELIMINARY\n\nBody of the letter goes on here.";
        let r = render(vec![Content::Ocr(letter.into())], Some("Real Metadata Title".into()), "521_BERS_2020_SHPRA.pdf");
        assert_eq!(r.title.as_deref(), Some("Evaluation of Sexual Harassment Prevention and Response Practices"));
        assert_eq!(r.date.as_deref(), Some("2020-06-09"));
        let none = render(vec![Content::Ocr("Some scanned words here for the page.".into())], None, "x.pdf");
        assert_eq!(none.date, None);
    }

    #[test]
    fn letterhead_is_not_a_title_in_large_type() {
        let line = |base: f32, size: f32, text: &str| Line { x0: 72.0, x1: 300.0, base, size, bold: false, text: text.into(), col: 0 };
        let mut lines = vec![line(740.0, 24.0, "BY EMAIL"), line(700.0, 20.0, "Annual Hiring Review")];
        for i in 0..12 {
            lines.push(line(600.0 - 14.0 * i as f32, 10.0, "Ordinary body text that runs along the line."));
        }
        let r = render(vec![Content::Lines(Page { height: 792.0, lines })], None, "x.pdf");
        assert_eq!(r.title.as_deref(), Some("Annual Hiring Review"));
    }
}
