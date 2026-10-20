//! Lists of downloads in and out: IDM's `.ef2` export format, and IDM's batch patterns.
//!
//! ## `.ef2`
//!
//! IDM's *Tasks › Export* writes, and *Import* reads, one record per download:
//!
//! ```text
//! <
//! https://example.com/file.zip
//! referer: https://example.com/
//! cookie: a=1; b=2
//! User-Agent: Mozilla/5.0 …
//! >
//! ```
//!
//! with CRLF line ends. This follows IDM's own reader (`CUrlExporter::ReadFromIEFFile` in
//! `IDMan.exe`): a record opens with `<\r\n`, closes at `\r\n>\r\n`, the first line is the
//! address, and `referer: `, `cookie: `, `pd: ` and `User-Agent: ` are looked for by name — the
//! tag strings are read straight from the binary's data section. `pd: ` is the body of a form
//! submission: such a download is a POST, which reIDM cannot repeat, so it is kept apart rather
//! than fetched with a GET that would get some other answer. The reader also accepts bare `\n`
//! line ends, because a file that has been through a text editor or `git` often has them.
//!
//! ## Batch patterns
//!
//! IDM's *Add batch download* takes an address with one `*` and a range — `img*.jpg`, 1 to 20,
//! zero-padded to 3 digits gives `img001.jpg` … `img020.jpg`; a letter range does the same with
//! `a` … `z`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// One download as an export file describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub url: String,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub cookie: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    /// The request body of a download that came from a form (`pd: `). Its presence means the
    /// download needs a POST.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_data: Option<String>,
}

/// Does this text look like an `.ef2` file rather than a plain list of URLs?
pub fn looks_like_ef2(text: &str) -> bool {
    text.lines().map(str::trim).find(|l| !l.is_empty()) == Some("<")
}

/// Read every record in an `.ef2` file. Records without a usable `http`/`https`/`ftp` address
/// are skipped rather than failing the whole import.
pub fn parse_ef2(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<&str>> = None;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        match (line.trim(), cur.as_mut()) {
            ("<", _) => cur = Some(Vec::new()),
            (">", Some(_)) => {
                if let Some(e) = cur.take().and_then(|lines| entry_from(&lines)) {
                    out.push(e);
                }
            }
            (_, Some(lines)) => lines.push(line),
            (_, None) => {} // outside a record: IDM ignores it too
        }
    }
    out
}

fn entry_from(lines: &[&str]) -> Option<Entry> {
    let url = lines.iter().map(|l| l.trim()).find(|l| !l.is_empty())?.to_string();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ftp://")) {
        return None;
    }
    let field = |name: &str| {
        lines.iter().find_map(|l| {
            let l = l.trim_start();
            if l.len() >= name.len() && l[..name.len()].eq_ignore_ascii_case(name) {
                Some(l[name.len()..].trim().to_string()).filter(|v| !v.is_empty())
            } else {
                None
            }
        })
    };
    Some(Entry { referer: field("referer:"), cookie: field("cookie:"), user_agent: field("User-Agent:"), post_data: field("pd:"), url })
}

/// Write records the way IDM does, so the file imports back into IDM as well as into reIDM.
///
/// Values containing a line break are dropped rather than written: a newline inside a field
/// would end the field early and the remainder would be read as a new one.
pub fn write_ef2(entries: &[Entry]) -> String {
    let mut s = String::new();
    let clean = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty() && !v.contains(['\r', '\n'])).map(str::to_string);
    for e in entries {
        if e.url.contains(['\r', '\n']) || e.url.trim().is_empty() {
            continue;
        }
        s.push_str("<\r\n");
        s.push_str(e.url.trim());
        s.push_str("\r\n");
        if let Some(r) = clean(&e.referer) {
            s.push_str(&format!("referer: {r}\r\n"));
        }
        if let Some(c) = clean(&e.cookie) {
            s.push_str(&format!("cookie: {c}\r\n"));
        }
        if let Some(u) = clean(&e.user_agent) {
            s.push_str(&format!("User-Agent: {u}\r\n"));
        }
        s.push_str(">\r\n");
    }
    s
}

/// What replaces the `*` in a batch pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sequence {
    /// `from..=to`, zero-padded to `width` digits (0 or 1 means no padding).
    Numbers {
        from: u64,
        to: u64,
        #[serde(default)]
        width: usize,
    },
    /// `from..=to` over ASCII letters of one case, e.g. `a`..`z`.
    Letters { from: char, to: char },
}

/// The most addresses one pattern may produce. A typo like `1` to `1000000` should be an error,
/// not a million queued downloads.
pub const MAX_PATTERN_ITEMS: usize = 10_000;

/// The widest zero-padding a number may have: the largest number has 20 digits, so wider only
/// costs memory.
pub const MAX_PATTERN_WIDTH: usize = 20;

/// Expand a pattern with exactly one `*`.
pub fn expand_pattern(pattern: &str, seq: &Sequence) -> Result<Vec<String>> {
    let pattern = pattern.trim();
    let stars = pattern.matches('*').count();
    if stars != 1 {
        bail!("the address needs exactly one * to replace (it has {stars})");
    }
    let (head, tail) = pattern.split_once('*').expect("one star");
    let items: Vec<String> = match *seq {
        Sequence::Numbers { from, to, width } => {
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            // Checked: `0` to `u64::MAX` is one more than a u64 holds, and an unchecked count
            // wrapped to 0, passed the size check and then never finished counting.
            let n = (hi - lo).checked_add(1);
            if n.is_none_or(|n| n > MAX_PATTERN_ITEMS as u64) {
                let n = n.map_or_else(|| "18446744073709551616".to_string(), |n| n.to_string());
                bail!("{lo} to {hi} is {n} addresses, more than the {MAX_PATTERN_ITEMS} a pattern may produce; use a smaller range");
            }
            if width > MAX_PATTERN_WIDTH {
                bail!("numbers can be padded to at most {MAX_PATTERN_WIDTH} digits (asked for {width})");
            }
            (lo..=hi).map(|i| format!("{i:0width$}", width = width.max(1))).collect()
        }
        Sequence::Letters { from, to } => {
            let same_case = (from.is_ascii_lowercase() && to.is_ascii_lowercase()) || (from.is_ascii_uppercase() && to.is_ascii_uppercase());
            if !same_case {
                bail!("a letter range must be a..z or A..Z in one case (got {from:?}..{to:?})");
            }
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            (lo..=hi).map(|c| c.to_string()).collect()
        }
    };
    Ok(items.into_iter().map(|x| format!("{head}{x}{tail}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDM_EXPORT: &str = "<\r\nhttps://example.com/a.zip\r\nreferer: https://example.com/page\r\nUser-Agent: Mozilla/5.0 (Windows NT 10.0)\r\n>\r\n<\r\nhttps://example.com/b.iso\r\ncookie: sid=1; x=2\r\n>\r\n";

    #[test]
    fn reads_idm_export_records() {
        assert!(looks_like_ef2(IDM_EXPORT));
        let v = parse_ef2(IDM_EXPORT);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].url, "https://example.com/a.zip");
        assert_eq!(v[0].referer.as_deref(), Some("https://example.com/page"));
        assert_eq!(v[0].user_agent.as_deref(), Some("Mozilla/5.0 (Windows NT 10.0)"));
        assert_eq!(v[0].cookie, None);
        assert_eq!(v[1].cookie.as_deref(), Some("sid=1; x=2"));
    }

    #[test]
    fn tolerates_lf_unknown_fields_and_junk() {
        let text = "junk before\n<\nhttps://x.example/f.bin\nx-unknown: something\nREFERER: https://x.example/\n>\n<\nnot a url\n>\n";
        let v = parse_ef2(text);
        assert_eq!(v.len(), 1, "the record without an address is skipped");
        assert_eq!(v[0].referer.as_deref(), Some("https://x.example/"));
        assert_eq!(v[0].post_data, None);
    }

    #[test]
    fn a_form_download_keeps_its_post_body() {
        let text = "<\r\nhttps://x.example/export\r\npd: q=1&format=csv\r\n>\r\n";
        let v = parse_ef2(text);
        assert_eq!(v[0].post_data.as_deref(), Some("q=1&format=csv"));
    }

    #[test]
    fn round_trips() {
        let entries = vec![
            Entry { url: "https://a.example/1.zip".into(), referer: Some("https://a.example/".into()), ..Default::default() },
            Entry { url: "https://a.example/2.zip".into(), cookie: Some("k=v".into()), user_agent: Some("UA/1".into()), ..Default::default() },
        ];
        let text = write_ef2(&entries);
        assert!(text.contains("\r\n>\r\n"), "IDM's reader looks for CRLF record ends");
        assert_eq!(parse_ef2(&text), entries);
    }

    #[test]
    fn a_newline_inside_a_value_is_not_written() {
        let e = Entry { url: "https://a.example/1.zip".into(), referer: Some("https://a.example/\r\ncookie: injected".into()), ..Default::default() };
        let back = parse_ef2(&write_ef2(&[e]));
        assert_eq!(back[0].referer, None);
        assert_eq!(back[0].cookie, None);
    }

    #[test]
    fn plain_lists_are_not_mistaken_for_ef2() {
        assert!(!looks_like_ef2("https://a.example/1.zip\n"));
        assert!(!looks_like_ef2(""));
    }

    #[test]
    fn number_patterns_pad_and_order() {
        let v = expand_pattern("https://x/img*.jpg", &Sequence::Numbers { from: 8, to: 11, width: 3 }).unwrap();
        assert_eq!(v, ["https://x/img008.jpg", "https://x/img009.jpg", "https://x/img010.jpg", "https://x/img011.jpg"]);
        // A reversed range is the same range.
        let r = expand_pattern("https://x/*.jpg", &Sequence::Numbers { from: 3, to: 1, width: 0 }).unwrap();
        assert_eq!(r, ["https://x/1.jpg", "https://x/2.jpg", "https://x/3.jpg"]);
    }

    #[test]
    fn letter_patterns() {
        let v = expand_pattern("https://x/part-*.rar", &Sequence::Letters { from: 'a', to: 'c' }).unwrap();
        assert_eq!(v, ["https://x/part-a.rar", "https://x/part-b.rar", "https://x/part-c.rar"]);
        assert!(expand_pattern("https://x/*", &Sequence::Letters { from: 'a', to: 'C' }).is_err());
    }

    #[test]
    fn patterns_need_one_star_and_a_sane_size() {
        assert!(expand_pattern("https://x/a.jpg", &Sequence::Numbers { from: 1, to: 2, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*/*.jpg", &Sequence::Numbers { from: 1, to: 2, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*.jpg", &Sequence::Numbers { from: 0, to: 1_000_000, width: 0 }).is_err());
    }

    #[test]
    fn huge_ranges_and_widths_are_refused_at_once() {
        // The whole of u64 used to wrap the count to zero and then expand without end.
        let e = expand_pattern("https://x/*", &Sequence::Numbers { from: 0, to: u64::MAX, width: 0 }).unwrap_err();
        assert!(e.to_string().contains("more than the 10000"), "{e}");
        assert!(expand_pattern("https://x/*", &Sequence::Numbers { from: u64::MAX, to: 1, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*", &Sequence::Numbers { from: 1, to: 2, width: usize::MAX }).is_err());
        // The boundaries themselves still work.
        assert_eq!(expand_pattern("https://x/*", &Sequence::Numbers { from: u64::MAX, to: u64::MAX, width: 20 }).unwrap(), ["https://x/18446744073709551615"]);
        assert_eq!(expand_pattern("https://x/*", &Sequence::Numbers { from: 1, to: 10_000, width: 0 }).unwrap().len(), 10_000);
    }
}
