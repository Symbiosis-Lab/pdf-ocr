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
}

pub struct Rendered {
    pub markdown: String,
    pub title: Option<String>,
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

pub fn ocr_to_markdown(text: &str, vocab: &Vocab) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for para in text.split("\n\n") {
        let mut cur: Vec<String> = Vec::new();
        for raw in para.lines() {
            let l = raw.trim();
            if l.is_empty() {
                continue;
            }
            let is_item = list_marker(l).is_some();
            if is_item || cur.is_empty() {
                cur.push(l.to_string());
            } else {
                let last = cur.last_mut().unwrap();
                join_wrapped(last, l, vocab);
            }
        }
        for item in cur {
            blocks.push(render_oneline(&item));
        }
    }
    blocks.join("\n\n")
}

fn render_oneline(s: &str) -> String {
    let (prefix, rest) = match list_marker(s) {
        Some((Item::Bullet, n)) => ("- ".to_string(), &s[n..]),
        Some((Item::Numbered, _)) => return escape_inline(s),
        _ => (String::new(), s),
    };
    format!("{prefix}{}", escape_line_start(&escape_inline(rest)))
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
            brk = !style || gap > para_gap * l.size.max(pl.size) || marker.is_some() || indent || gap <= 0.0;
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
            out.push(Blk { kind, logical: vec![text_after(marker)], size: l.size, bold: l.bold, ends_open: false });
        } else {
            let b = out.last_mut().unwrap();
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
        b.ends_open = !t.ends_with(['.', '!', '?', ':', ';', '"', '”', ')']);
    }
    out
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

pub fn render(contents: Vec<Content>, meta_title: Option<String>) -> Rendered {
    // 1. running headers/footers over the text pages
    let mut contents = contents;
    {
        let mut refs: Vec<&mut Page> = contents
            .iter_mut()
            .filter_map(|c| if let Content::Lines(p) = c { Some(p) } else { None })
            .collect();
        strip_running(&mut refs);
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
    let mut ocr_flag: Vec<Option<String>> = Vec::new();
    for c in &contents {
        match c {
            Content::Lines(p) => {
                let mut bl = blocks_for_page(p, para_gap, &vocab);
                for b in bl.iter_mut() {
                    if classify_heading(b, body) {
                        b.kind = Kind::Heading;
                    }
                }
                page_blocks.push(bl);
                ocr_flag.push(None);
            }
            Content::Ocr(t) => {
                page_blocks.push(Vec::new());
                ocr_flag.push(Some(ocr_to_markdown(t, &vocab)));
            }
        }
    }

    // 3. title from the first text page
    let mut title: Option<String> = None;
    let mut drop_title: Option<(usize, usize)> = None;
    if let Some((pi, bl)) = page_blocks.iter().enumerate().find(|(i, b)| ocr_flag[*i].is_none() && !b.is_empty()) {
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

    // 5. flatten, merge paragraphs across page breaks, render
    let mut parts: Vec<(String, bool)> = Vec::new(); // (markdown, tight-with-previous)
    let mut carry: Option<Blk> = None; // last paragraph of previous text page awaiting a continuation
    let flush = |carry: &mut Option<Blk>, parts: &mut Vec<(String, bool)>| {
        if let Some(b) = carry.take() {
            parts.push((render_block(&b, 0), false));
        }
    };
    let mut prev_item = false;
    for (pi, bl) in page_blocks.iter().enumerate() {
        if let Some(t) = ocr_flag[pi].as_ref() {
            flush(&mut carry, &mut parts);
            if !t.is_empty() {
                parts.push((t.clone(), false));
            }
            prev_item = false;
            continue;
        }
        let n = bl.len();
        for (bi, b) in bl.iter().enumerate() {
            if drop_title == Some((pi, bi)) {
                continue;
            }
            let mut b = b.clone();
            if let Some(mut c) = carry.take() {
                let starts_lower = b.logical[0].chars().next().is_some_and(|ch| ch.is_lowercase());
                if bi == 0 && b.kind == Kind::Para && starts_lower && c.ends_open {
                    let first = b.logical.remove(0);
                    let last = c.logical.last_mut().unwrap();
                    join_wrapped(last, &first, &vocab);
                    c.logical.extend(b.logical.drain(..));
                    c.ends_open = b.ends_open;
                    b = c;
                } else {
                    parts.push((render_block(&c, 0), false));
                }
            }
            let last_on_page = bi + 1 == n;
            if last_on_page && b.kind == Kind::Para && b.ends_open {
                carry = Some(b);
                continue;
            }
            let lvl = if b.kind == Kind::Heading { level(&b) } else { 0 };
            let is_item = matches!(b.kind, Kind::Item(Item::Bullet) | Kind::Item(Item::Numbered));
            let tight = is_item && prev_item;
            prev_item = is_item;
            parts.push((render_block(&b, lvl), tight));
        }
    }
    flush(&mut carry, &mut parts);

    let mut md = String::new();
    for (i, (s, tight)) in parts.iter().enumerate() {
        if i > 0 {
            md.push_str(if *tight { "\n" } else { "\n\n" });
        }
        md.push_str(s);
    }
    Rendered { markdown: md, title }
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
        let mk = |i: usize, w: &str| Line { x0: 72.0, x1: 540.0, base: 700.0 - 24.0 * i as f32, size: 12.0, bold: false, text: w.to_string(), tabular: false };
        let mut lines: Vec<Line> = (0..6).map(|i| mk(i, "word word word word word word word word word word word word")).collect();
        lines.push(Line { x1: 300.0, ..mk(6, "last line of paragraph one.") });
        lines.extend((8..12).map(|i| mk(i, "second paragraph words words words words words words words")));
        let r = render(vec![Content::Lines(Page { height: 792.0, lines })], None);
        assert_eq!(r.markdown.split("\n\n").count(), 2, "{}", r.markdown);
    }
}
