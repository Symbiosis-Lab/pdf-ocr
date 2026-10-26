//! What a first page states about itself: a letter's subject line and its date,
//! plus the letterhead lines that must not pass for a title. Pure text rules,
//! the same for a text layer and for OCR.

const LABELS: [&str; 3] = ["audit status", "subject", "re"];

/// Where a subject ends and the audit particulars, the finding or the letter begin.
const CUTS: [&str; 5] = ["determination:", " for audit period", " for the audit period", " for the period", " for period"];

const TITLE_CAP: usize = 100;
const ABBREVIATIONS: [&str; 9] = ["inc", "corp", "dept", "assn", "ave", "bldg", "mrs", "vs", "etc"];
const CONNECTORS: [&str; 11] = ["of", "and", "the", "for", "to", "in", "on", "a", "an", "with", "or"];

fn after_label(s: &str) -> Option<&str> {
    LABELS.iter().find_map(|label| {
        let head = s.get(..label.len())?;
        let rest = s[label.len()..].trim_start();
        (head.eq_ignore_ascii_case(label) && rest.starts_with(':')).then(|| rest[1..].trim())
    })
}

/// The text after a subject label that opens `line`, or, with `mid`, one that
/// follows whitespace inside it (OCR joins a paragraph's lines into one).
fn subject_text(line: &str, mid: bool) -> Option<&str> {
    let line = line.trim();
    if !mid {
        return after_label(line);
    }
    line.char_indices()
        .filter(|(i, _)| *i > 0 && line[..*i].ends_with(char::is_whitespace))
        .find_map(|(i, _)| after_label(&line[i..]))
}

/// The subject a first page states under a "Subject:", "Re:" or "Audit Status:"
/// label, trimmed to the subject itself. `blocks` are the page's text blocks in
/// reading order, each a list of lines.
pub fn subject_title(blocks: &[&[String]]) -> Option<String> {
    for mid in [false, true] {
        for (bi, block) in blocks.iter().enumerate() {
            for (li, line) in block.iter().enumerate() {
                let Some(rest) = subject_text(line, mid) else { continue };
                let mut text = rest.to_string();
                for l in &block[li + 1..] {
                    text.push(' ');
                    text.push_str(l);
                }
                if rest.is_empty() {
                    if let Some(next) = blocks.get(bi + 1) {
                        text.push(' ');
                        text.push_str(&next.join(" "));
                    }
                }
                if let Some(t) = trim_subject(&text) {
                    return Some(t);
                }
            }
        }
    }
    None
}

fn trim_subject(text: &str) -> Option<String> {
    let mut s = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = s.to_ascii_lowercase();
    if let Some(at) = CUTS.iter().filter_map(|c| lower.find(c)).min() {
        s.truncate(at);
    }
    if let Some(at) = sentence_end(&s) {
        s.truncate(at);
    }
    let edge = |c: char| c.is_whitespace() || ".,;:-\u{2013}\u{2014}".contains(c);
    let mut s = s.trim_matches(edge).to_string();
    if s.chars().count() > TITLE_CAP {
        let cut = s.char_indices().nth(TITLE_CAP).map_or(s.len(), |(i, _)| i);
        let mut words: Vec<&str> = s[..cut].split(' ').collect();
        // The last word may be cut mid-way unless the cap fell on a space.
        if !s[cut..].starts_with(' ') {
            words.pop();
        }
        while words.last().is_some_and(|w| CONNECTORS.contains(&w.to_ascii_lowercase().as_str())) {
            words.pop();
        }
        s = format!("{}\u{2026}", words.join(" ").trim_end_matches(edge));
    }
    (s.chars().count() >= 3 && s.chars().any(char::is_alphabetic)).then_some(s)
}

/// Index of the first full stop, question or exclamation mark that ends a
/// sentence: followed by a capital (or nothing), and not closing an abbreviation.
fn sentence_end(s: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    chars.iter().enumerate().find_map(|(k, &(i, c))| {
        if !matches!(c, '.' | '?' | '!') {
            return None;
        }
        let next = chars[k + 1..].iter().map(|&(_, c)| c).find(|c| !c.is_whitespace());
        let spaced = chars.get(k + 1).is_none_or(|&(_, c)| c.is_whitespace());
        let word: String = s[..i].chars().rev().take_while(|c| c.is_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
        let abbreviation = c == '.' && (word.chars().count() < 3 || ABBREVIATIONS.contains(&word.to_ascii_lowercase().as_str()));
        (spaced && !abbreviation && next.is_none_or(char::is_uppercase)).then_some(i)
    })
}

// ---------- letterhead ----------

const STREET_WORDS: [&str; 14] = [
    "street", "st", "avenue", "ave", "floor", "fl", "broadway", "plaza", "place", "road", "rd", "suite", "centre", "center",
];

/// A letterhead line: delivery method, agency name, street address, phone or fax, web address, or a lone date.
pub fn is_letterhead(text: &str) -> bool {
    let l = text.trim().to_ascii_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let delivery = ["by email", "by hand", "by fax", "by mail", "by e-mail", "via email", "via hand", "via fax", "via mail"];
    if delivery.iter().any(|d| l.starts_with(d)) {
        return true;
    }
    if l.contains('@') || l.starts_with("www.") || l.starts_with("http") {
        return true;
    }
    let agency = l.replace("equal employment practices commission", "");
    if agency != l {
        let rest: Vec<&str> = agency.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        if rest.iter().all(|w| ["nyc", "new", "york", "city", "of", "the", "ny", "eepc"].contains(w)) {
            return true;
        }
    }
    if words == ["nyc"] || words == ["new", "york", "city"] {
        return true;
    }
    let digits = l.chars().filter(char::is_ascii_digit).count();
    let phone_lead = ["tel", "phone", "fax", "t:", "f:", "telephone"].iter().any(|p| l.starts_with(p));
    if (phone_lead && digits >= 7) || (digits >= 7 && l.chars().all(|c| c.is_ascii_digit() || " ()-+.x/".contains(c))) {
        return true;
    }
    let street = words.first().is_some_and(|w| w.chars().all(|c| c.is_ascii_digit()) || w.ends_with("th") || w.ends_with("st"))
        && words.iter().any(|w| STREET_WORDS.contains(w));
    if street && words.len() <= 10 {
        return true;
    }
    if l.contains("new york, ny") || l.contains("new york ny") || has_zip(&l) {
        return words.len() <= 10;
    }
    dates_only(text)
}

fn has_zip(l: &str) -> bool {
    l.split_whitespace().any(|t| {
        let t = t.trim_matches(|c: char| !c.is_ascii_digit());
        t.len() == 5 && t.chars().all(|c| c.is_ascii_digit()) && l.contains("ny")
    })
}

fn dates_only(text: &str) -> bool {
    let toks = tokens(text);
    dates(&toks).first().is_some_and(|d| d.start == 0 && d.end == toks.len())
}

// ---------- dates ----------

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

struct Found {
    start: usize,
    end: usize,
    iso: String,
}

fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn clean(t: &str) -> &str {
    t.trim_matches(|c: char| ",.;:()[]\"'".contains(c))
}

fn month_of(t: &str) -> Option<u32> {
    let t = clean(t).to_ascii_lowercase();
    let t = t.as_str();
    let m = MONTHS.iter().position(|m| *m == t || (t.len() >= 3 && m.starts_with(t) && (t.len() == 3 || (t == "sept" && *m == "september"))));
    m.map(|i| i as u32 + 1)
}

fn day_of(t: &str) -> Option<u32> {
    let t = clean(t);
    let digits = t.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &t[digits.len()..];
    let ok = suffix.is_empty() || ["st", "nd", "rd", "th"].contains(&suffix.to_ascii_lowercase().as_str());
    (ok && (1..=2).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
}

fn year_of(t: &str) -> Option<u32> {
    let t = clean(t);
    (t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok()).flatten().filter(|y| (1900..=2100).contains(y))
}

fn iso(y: u32, m: u32, d: u32) -> Option<String> {
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + u32::from(leap),
        _ => return None,
    };
    (1..=days).contains(&d).then(|| format!("{y:04}-{m:02}-{d:02}"))
}

fn numeric_date(t: &str) -> Option<String> {
    let t = clean(t);
    if let [y, m, d] = t.split('-').collect::<Vec<_>>()[..] {
        if y.len() == 4 && m.len() == 2 && d.len() == 2 {
            return iso(year_of(y)?, m.parse().ok()?, d.parse().ok()?);
        }
    }
    if let [m, d, y] = t.split('/').collect::<Vec<_>>()[..] {
        if (1..=2).contains(&m.len()) && (1..=2).contains(&d.len()) {
            return iso(year_of(y)?, m.parse().ok()?, d.parse().ok()?);
        }
    }
    None
}

/// Every full date in the token stream, with the tokens it spans.
fn dates(toks: &[String]) -> Vec<Found> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let found = if let Some(iso) = numeric_date(&toks[i]) {
            Some((1, iso))
        } else if let (Some(m), Some(d), Some(y)) = (month_of(&toks[i]), toks.get(i + 1).and_then(|t| day_of(t)), toks.get(i + 2).and_then(|t| year_of(t))) {
            iso(y, m, d).map(|s| (3, s))
        } else if let (Some(d), Some(m), Some(y)) = (day_of(&toks[i]), toks.get(i + 1).and_then(|t| month_of(t)), toks.get(i + 2).and_then(|t| year_of(t))) {
            iso(y, m, d).map(|s| (3, s))
        } else {
            None
        };
        match found {
            Some((n, iso)) => {
                out.push(Found { start: i, end: i + n, iso });
                i += n;
            }
            None => i += 1,
        }
    }
    out
}

fn is_range_word(t: &str) -> bool {
    matches!(clean(t).to_ascii_lowercase().as_str(), "to" | "through" | "thru" | "-" | "\u{2013}" | "\u{2014}" | "until")
}

/// The first full date a first page states, as `YYYY-MM-DD`, leaving out the
/// dates of an audit period or any other "X to Y" range.
pub fn first_date(blocks: &[&[String]]) -> Option<String> {
    let text: Vec<&str> = blocks.iter().flat_map(|b| b.iter().map(String::as_str)).collect();
    let toks = tokens(&text.join(" "));
    let found = dates(&toks);
    found.iter().enumerate().find_map(|(k, d)| {
        let before = &toks[d.start.saturating_sub(6)..d.start];
        let in_period = before.iter().any(|t| clean(t).eq_ignore_ascii_case("period"));
        let after_range = d.start > 0 && is_range_word(&toks[d.start - 1]) && k > 0 && found[k - 1].end + 1 >= d.start;
        let before_range = toks.get(d.end).is_some_and(|t| is_range_word(t)) && found.get(k + 1).is_some_and(|n| n.start <= d.end + 1);
        (!(in_period || after_range || before_range)).then(|| d.iso.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(b: &[&[&str]]) -> Vec<Vec<String>> {
        b.iter().map(|b| b.iter().map(|s| s.to_string()).collect()).collect()
    }

    fn subject(b: &[&[&str]]) -> Option<String> {
        let owned = blocks(b);
        let refs: Vec<&[String]> = owned.iter().map(Vec::as_slice).collect();
        subject_title(&refs)
    }

    fn date(b: &[&[&str]]) -> Option<String> {
        let owned = blocks(b);
        let refs: Vec<&[String]> = owned.iter().map(Vec::as_slice).collect();
        first_date(&refs)
    }

    #[test]
    fn subject_labels_take_the_text_after_them() {
        for (line, want) in [
            ("Re: Annual Hiring Review", "Annual Hiring Review"),
            ("RE: Annual Hiring Review", "Annual Hiring Review"),
            ("Subject: Annual Hiring Review", "Annual Hiring Review"),
            ("SUBJECT:   Annual Hiring Review", "Annual Hiring Review"),
            ("Re : Annual Hiring Review", "Annual Hiring Review"),
            ("Audit Status: Annual Hiring Review", "Annual Hiring Review"),
        ] {
            assert_eq!(subject(&[&["Dear Chair"], &[line]]).as_deref(), Some(want), "{line}");
        }
        assert_eq!(subject(&[&["Are: not a label", "Reason: none"]]), None);
    }

    #[test]
    fn the_subject_is_cut_before_audit_period_determination_and_the_sentence_end() {
        let full = "Audit Status: Evaluation of Sexual Harassment Prevention and Response Practices for Audit Period January 1, 2018 to December 31, 2019. Determination: PRELIMINARY";
        assert_eq!(subject(&[&[full]]).as_deref(), Some("Evaluation of Sexual Harassment Prevention and Response Practices"));
        assert_eq!(subject(&[&["Subject: Hiring Practices for the period 2018. Determination: FINAL"]]).as_deref(), Some("Hiring Practices"));
        assert_eq!(subject(&[&["Re: Hiring Practices Determination: FINAL"]]).as_deref(), Some("Hiring Practices"));
        assert_eq!(subject(&[&["Re: Hiring Practices. Please find the report attached."]]).as_deref(), Some("Hiring Practices"));
        assert_eq!(subject(&[&["Re: Acme Inc. Hiring Practices. Thank you."]]).as_deref(), Some("Acme Inc. Hiring Practices"));
    }

    #[test]
    fn a_wrapped_subject_is_joined_and_a_label_alone_takes_the_next_block() {
        assert_eq!(subject(&[&["Audit Status: Evaluation of Sexual", "Harassment Prevention"]]).as_deref(), Some("Evaluation of Sexual Harassment Prevention"));
        assert_eq!(subject(&[&["Audit Status:"], &["Evaluation of Hiring for Audit Period 2018 to 2019."], &["PRELIMINARY"]]).as_deref(), Some("Evaluation of Hiring"));
    }

    #[test]
    fn an_ocr_paragraph_may_carry_the_label_mid_text() {
        assert_eq!(subject(&[&["Chair Jane Doe 1 Centre Street Re: Hiring Review for Audit Period 2018 to 2019"]]).as_deref(), Some("Hiring Review"));
    }

    #[test]
    fn a_long_subject_is_capped_at_a_word() {
        let long = format!("Re: {}", "Evaluation of hiring and promotion practices across every division of the city ".repeat(3));
        let t = subject(&[&[&long]]).unwrap();
        assert!(t.chars().count() <= TITLE_CAP + 1 && t.ends_with('\u{2026}'), "{t}");
        let body = t.trim_end_matches('\u{2026}');
        assert!(long[4..].starts_with(body) && !body.ends_with(' ') && !body.ends_with(" of"), "{t}");
    }

    #[test]
    fn no_label_or_an_empty_subject_gives_none() {
        assert_eq!(subject(&[&["Memo", "To: the Chair"]]), None);
        assert_eq!(subject(&[&["Re: ."]]), None);
    }

    #[test]
    fn letterhead_is_recognised_and_real_titles_are_not() {
        for l in [
            "BY EMAIL", "BY HAND", "By Fax and Mail", "Equal Employment Practices Commission", "NYC", "NYC Equal Employment Practices Commission",
            "253 Broadway, 6th Floor", "1 Centre Street, Room 650", "New York, NY 10007", "Tel: (212) 442-0000", "(212) 442-0000", "www.nyc.gov/eepc",
            "June 9, 2020", "06/09/2020",
        ] {
            assert!(is_letterhead(l), "{l}");
        }
        for l in [
            "521 BERS 2020 SHPRA", "NYC EEPC presentation Outline", "Equal Employment Practices Commission 2022 Annual Report",
            "Meeting Agenda", "2022 Annual Report", "Audit of Hiring, June 9, 2020 Edition",
        ] {
            assert!(!is_letterhead(l), "{l}");
        }
    }

    #[test]
    fn each_date_format_becomes_an_iso_date() {
        for (text, want) in [
            ("June 9, 2020", "2020-06-09"),
            ("January 17, 2002", "2002-01-17"),
            ("Sept. 3rd, 2019", "2019-09-03"),
            ("Dec 25 2018", "2018-12-25"),
            ("9 June 2020", "2020-06-09"),
            ("06/09/2020", "2020-06-09"),
            ("6/9/2020", "2020-06-09"),
            ("2020-06-09", "2020-06-09"),
            ("Dated Tuesday, March 3, 2020, at noon", "2020-03-03"),
        ] {
            assert_eq!(date(&[&["Letterhead", text]]).as_deref(), Some(want), "{text}");
        }
    }

    #[test]
    fn the_first_date_wins_and_period_ranges_are_ignored() {
        assert_eq!(date(&[&["March 3, 2020"], &["May 5, 2020"]]).as_deref(), Some("2020-03-03"));
        assert_eq!(date(&[&["Audit Status: Hiring for Audit Period January 1, 2018 to December 31, 2019."], &["June 9, 2020"]]).as_deref(), Some("2020-06-09"));
        assert_eq!(date(&[&["Audit Period January 1, 2018 to December 31, 2019"]]), None);
        assert_eq!(date(&[&["from January 1, 2018 to", "December 31, 2019"]]), None);
        assert_eq!(date(&[&["January 1, 2018 - December 31, 2019"]]), None);
    }

    #[test]
    fn no_full_date_gives_none() {
        for text in ["2022 Annual Report", "June 2020", "March 3", "13/45/2020", "February 30, 2020", "06/09/20", "May 5, 1850"] {
            assert_eq!(date(&[&[text]]), None, "{text}");
        }
    }
}
