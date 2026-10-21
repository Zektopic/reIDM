//! Site grabber: find the downloadable files on a page, optionally following links.
//!
//! Deliberately dependency-free on the parsing side — a download manager only needs the URLs
//! out of a page, and a full HTML5 parser is a large amount of code to pull in for that.
//! The extractor handles the attribute forms pages actually use, resolves relative URLs against
//! the post-redirect base, and decodes the entities that appear inside attributes.
//!
//! It is not an HTML validator: script-generated links and CSS-referenced assets are invisible
//! to it, and that limitation is reported rather than hidden.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use url::Url;

/// What to collect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    /// 0 = only the page you were given; 1 = also follow its links once, and so on.
    #[serde(default)]
    pub depth: u32,
    /// Lower-case extensions without the dot. Empty means "everything".
    #[serde(default)]
    pub types: Vec<String>,
    /// Only keep links on the starting host.
    #[serde(default)]
    pub same_host_only: bool,
    /// Keep only URLs containing one of these strings (empty = keep all).
    #[serde(default)]
    pub include: Vec<String>,
    /// Drop URLs containing any of these strings.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Ask each server for the file size. Costs one request per hit, so it is opt-in.
    #[serde(default)]
    pub sizes: bool,
    /// Upper bound on pages fetched while walking, to keep a crawl finite.
    #[serde(default = "default_max_pages")]
    pub max_pages: u32,
}

fn default_max_pages() -> u32 {
    50
}

impl Default for Options {
    fn default() -> Self {
        Self {
            depth: 0,
            types: Vec::new(),
            same_host_only: false,
            include: Vec::new(),
            exclude: Vec::new(),
            sizes: false,
            max_pages: default_max_pages(),
        }
    }
}

/// One downloadable thing found on a page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Found {
    pub url: String,
    /// The name it would be saved as, derived from the URL path.
    pub filename: String,
    /// Lower-case extension without the dot, if the URL has one.
    pub ext: String,
    /// From `Content-Length`, when it was asked for and the server provided it.
    pub size: Option<u64>,
    /// The page the link was found on.
    pub found_on: String,
}

/// The default extension vocabulary, matching the categories the GUI shows.
pub const COMMON_TYPES: &[&str] = &[
    // video
    "3gp", "asf", "avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "rm", "ts", "vob", "webm", "wmv",
    // audio
    "aac", "aif", "flac", "m4a", "mp3", "ogg", "opus", "ra", "wav", "wma",
    // documents
    "chm", "doc", "docx", "epub", "mobi", "odt", "pdf", "ppt", "pptx", "rtf", "txt", "xls", "xlsx",
    // archives
    "7z", "ace", "arj", "bz2", "gz", "rar", "tar", "tgz", "xz", "zip", "zst",
    // programs and disk images
    "apk", "appimage", "bin", "deb", "dmg", "exe", "img", "iso", "msi", "pkg", "rpm",
];

/// Resolve the HTML entities that turn up inside attribute values.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] != '&' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let Some(semi) = b[i..].iter().position(|c| *c == ';').filter(|p| *p <= 12) else {
            out.push(b[i]);
            i += 1;
            continue;
        };
        let name: String = b[i + 1..i + semi].iter().collect();
        let decoded = match name.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => None,
        }
        .or_else(|| {
            // Numeric references: &#38; and &#x26;
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        });
        match decoded {
            Some(c) => {
                out.push(c);
                i += semi + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// Pull `href`/`src`-style attribute values out of a page, plus any `<base href>`.
///
/// Returns `(links, links_that_were_script_or_css_only)`. The second list is not used for
/// filtering; it exists so callers can tell a page with genuinely no links from one whose links
/// are built by JavaScript, which this cannot see.
pub fn extract_links(html: &str) -> (Vec<String>, bool) {
    const ATTRS: [&str; 6] = ["href=", "src=", "data-src=", "data-href=", "poster=", "content="];
    let mut links = Vec::new();
    let lower = html.to_ascii_lowercase();
    let clean = blank_non_markup(html);
    let bytes: Vec<char> = clean.chars().collect();
    let lb: Vec<char> = clean.to_ascii_lowercase().chars().collect();

    // Made once, not at every character of the page.
    let attrs: Vec<Vec<char>> = ATTRS.iter().map(|a| a.chars().collect()).collect();
    let mut i = 0usize;
    while i < lb.len() {
        let mut matched = None;
        for ac in &attrs {
            if i + ac.len() <= lb.len() && lb[i..i + ac.len()] == ac[..] {
                matched = Some(ac.len());
                break;
            }
        }
        let Some(alen) = matched else {
            i += 1;
            continue;
        };
        // Must be a whole attribute name, not the tail of a longer one.
        if i > 0 && (lb[i - 1].is_ascii_alphanumeric() || lb[i - 1] == '-' || lb[i - 1] == '_') {
            i += 1;
            continue;
        }
        let mut j = i + alen;
        while j < lb.len() && lb[j].is_whitespace() {
            j += 1;
        }
        if j >= lb.len() {
            break;
        }
        let (value, next) = match lb[j] {
            '"' | '\'' => {
                let quote = lb[j];
                let start = j + 1;
                let mut k = start;
                while k < lb.len() && lb[k] != quote {
                    k += 1;
                }
                (bytes[start..k.min(bytes.len())].iter().collect::<String>(), k + 1)
            }
            _ => {
                // Unquoted: runs to whitespace or the closing bracket.
                let start = j;
                let mut k = start;
                while k < lb.len() && !lb[k].is_whitespace() && lb[k] != '>' {
                    k += 1;
                }
                (bytes[start..k].iter().collect::<String>(), k)
            }
        };
        let v = decode_entities(value.trim());
        if !v.is_empty() {
            links.push(v);
        }
        i = next.max(i + 1);
    }

    // A page that mentions no links in its markup but does load a script is probably building
    // its links at runtime.
    let script_only = links.is_empty() && (lower.contains("<script") || lower.contains("fetch("));
    (links, script_only)
}

/// Blank out `<script>`, `<style>` and comment regions, tags included.
///
/// Without this, an inline JSON blob or a stylesheet full of `url(...)` produces a flood of
/// "links" that were never clickable. URLs that only ever exist inside a script are the browser
/// extension's job, since it can watch the requests the player actually makes.
fn blank_non_markup(html: &str) -> String {
    // `to_ascii_lowercase` only rewrites ASCII bytes, so offsets stay aligned with `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    const REGIONS: [(&str, &str); 3] = [("<script", "</script>"), ("<style", "</style>"), ("<!--", "-->")];
    // Where each opener next occurs (absolute offsets), searched again only once passed: a
    // fresh search of the rest of the page for every region made long pages quadratic.
    let find = |from: usize, open: &str| lower[from..].find(open).map(|p| from + p);
    let mut next: [Option<usize>; 3] = REGIONS.map(|(open, _)| find(0, open));
    let mut i = 0usize;
    while i < html.len() {
        for (k, (open, _)) in REGIONS.iter().enumerate() {
            if next[k].is_some_and(|p| p < i) {
                next[k] = find(i, open);
            }
        }
        let best = (0..REGIONS.len()).filter_map(|k| next[k].map(|p| (p, k))).min();
        match best {
            None => {
                out.push_str(&html[i..]);
                break;
            }
            Some((p, k)) => {
                let (open, closer) = REGIONS[k];
                out.push_str(&html[i..p]);
                let body = p + open.len();
                let end = lower[body..].find(closer).map(|c| body + c + closer.len()).unwrap_or(html.len());
                out.push_str(&" ".repeat(end - p));
                i = end;
            }
        }
    }
    out
}

/// `<base href="...">` overrides how relative links resolve.
fn base_href(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let at = lower.find("<base")?;
    let rest = &html[at..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let (links, _) = extract_links(tag);
    links.into_iter().next()
}

fn extension_of(u: &Url) -> String {
    let last = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, e)) if !e.is_empty() && e.len() <= 8 => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn filename_of(u: &Url) -> String {
    let last = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(last).decode_utf8_lossy().into_owned();
    if decoded.is_empty() { "download".into() } else { decoded }
}

fn keep(u: &Url, opts: &Options, start_host: &str) -> bool {
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    if opts.same_host_only && u.host_str().unwrap_or("") != start_host {
        return false;
    }
    let s = u.as_str();
    // "Containing one of these": any one pattern is enough.
    let include: Vec<&String> = opts.include.iter().filter(|p| !p.is_empty()).collect();
    if !include.is_empty() && !include.iter().any(|p| s.contains(p.as_str())) {
        return false;
    }
    if opts.exclude.iter().any(|p| !p.is_empty() && s.contains(p.as_str())) {
        return false;
    }
    true
}

/// Is this URL a file we would want, rather than another page?
fn looks_like_file(u: &Url, opts: &Options) -> bool {
    let ext = extension_of(u);
    if opts.types.is_empty() {
        // No filter: treat anything with a file-ish extension as a file, and bare paths as pages.
        return !ext.is_empty() && COMMON_TYPES.contains(&ext.as_str());
    }
    !ext.is_empty() && opts.types.iter().any(|t| t.eq_ignore_ascii_case(&ext))
}

/// Build a client and walk `start`. Convenience for callers that do not already have one.
pub async fn grab_page(start: &str, opts: &Options) -> Result<Vec<Found>> {
    grab_through(start, opts, &crate::Proxy::default()).await
}

/// [`grab_page`], connecting the way `proxy` says.
pub async fn grab_through(start: &str, opts: &Options, proxy: &crate::Proxy) -> Result<Vec<Found>> {
    let b = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(concat!("reIDM/", env!("CARGO_PKG_VERSION")));
    let client = proxy.configure(b)?.build().context("building the HTTP client")?;
    grab(&client, start, opts).await
}

/// Walk `start` and collect what is downloadable.
pub async fn grab(client: &reqwest::Client, start: &str, opts: &Options) -> Result<Vec<Found>> {
    let start_url = Url::parse(start).context("invalid start URL")?;
    if !matches!(start_url.scheme(), "http" | "https") {
        anyhow::bail!("only http and https pages can be grabbed");
    }
    let start_host = start_url.host_str().unwrap_or("").to_string();

    let mut seen_pages: HashSet<String> = HashSet::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut out: Vec<Found> = Vec::new();
    let mut queue: Vec<(Url, u32)> = vec![(start_url.clone(), 0)];
    let mut truncation_noted = false;

    while let Some((page, depth)) = queue.pop() {
        if seen_pages.len() >= opts.max_pages as usize {
            truncation_noted = true;
            break;
        }
        if !seen_pages.insert(page.as_str().to_string()) {
            continue;
        }

        let resp = match fetch(client, &page).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("grab: skipping {page}: {e:#}");
                continue;
            }
        };
        // Redirects change how relative links resolve.
        let base = base_href(&resp.body).and_then(|h| page.join(&h).ok()).unwrap_or_else(|| resp.final_url.clone());
        let (links, script_only) = extract_links(&resp.body);

        for raw in links {
            let Ok(u) = base.join(raw.trim()) else { continue };
            let u = strip_fragment(u);
            if !keep(&u, opts, &start_host) {
                continue;
            }
            if looks_like_file(&u, opts) {
                if seen_files.insert(u.as_str().to_string()) {
                    out.push(Found {
                        filename: filename_of(&u),
                        ext: extension_of(&u),
                        size: None,
                        url: u.as_str().to_string(),
                        found_on: page.as_str().to_string(),
                    });
                }
            } else if depth < opts.depth && !seen_pages.contains(u.as_str()) {
                queue.push((u, depth + 1));
            }
        }

        if script_only && depth == 0 {
            tracing::debug!("grab: {page} has no links in its markup; they may be built by script");
        }
    }

    if truncation_noted {
        tracing::warn!("grab: stopped after {} pages; raise max_pages or narrow the filters", opts.max_pages);
    }

    if opts.sizes {
        fill_sizes(client, &mut out).await;
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

fn strip_fragment(mut u: Url) -> Url {
    u.set_fragment(None);
    u
}

/// The largest page the grabber reads.
const MAX_PAGE: usize = 8 << 20;

struct Fetched {
    body: String,
    final_url: Url,
}

async fn fetch(client: &reqwest::Client, u: &Url) -> Result<Fetched> {
    let resp = client
        .get(u.clone())
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8")
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    let final_url = resp.url().clone();
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    // Refuse anything enormous; we only want the markup. Counted while reading too: a chunked
    // page says nothing of its size up front, and one that never ends must not fill the memory.
    let bytes = crate::torrent::read_capped(resp, MAX_PAGE, Duration::from_secs(20), "the page").await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Fetched { body, final_url })
}

/// Ask each server for the size, a few at a time. Failures are left as `None`.
async fn fill_sizes(client: &reqwest::Client, found: &mut [Found]) {
    let urls: Vec<String> = found.iter().map(|f| f.url.clone()).collect();
    let sizes: Vec<Option<u64>> = futures_util::stream::iter(urls.into_iter().map(|u| {
        let client = client.clone();
        async move {
            let r = client.head(&u).timeout(Duration::from_secs(10)).send().await.ok()?;
            if !r.status().is_success() {
                return None;
            }
            // Read the header rather than `content_length()`: for a HEAD response reqwest
            // reports the length of the (empty) body, which is always zero.
            r.headers()
                .get(reqwest::header::CONTENT_LENGTH)?
                .to_str()
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        }
    }))
    .buffered(6)
    .collect()
    .await;
    for (f, s) in found.iter_mut().zip(sizes) {
        f.size = s;
    }
}

/// Parse a list of URLs out of a text file or pasted block: one per line, `#` comments and blank
/// lines ignored, surrounding whitespace trimmed.
pub fn parse_url_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.starts_with("http://") || l.starts_with("https://") || l.starts_with("ftp://"))
        .map(str::to_string)
        .collect()
}

/// Render a list of URLs for export, with a comment header.
pub fn url_list_text(urls: &[String]) -> String {
    let mut s = String::from("# reIDM download list\n");
    for u in urls {
        s.push_str(u);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_attribute_forms_pages_use() {
        let html = r##"
            <html><head><base href="https://cdn.example/files/"></head>
            <body>
              <a href="one.zip">one</a>
              <a href='two.tar.gz'>two</a>
              <img src="/img/pic.png">
              <video><source src="clip.mp4" type="video/mp4"></video>
              <a href=unquoted.iso>three</a>
              <a href="https://other.example/x.deb?a=1&amp;b=2">four</a>
              <a href="#anchor">skip</a>
            </body></html>"##;
        let (links, _) = extract_links(html);
        assert!(links.contains(&"one.zip".to_string()));
        assert!(links.contains(&"two.tar.gz".to_string()));
        assert!(links.contains(&"/img/pic.png".to_string()));
        assert!(links.contains(&"clip.mp4".to_string()));
        assert!(links.contains(&"unquoted.iso".to_string()));
        assert!(links.contains(&"https://other.example/x.deb?a=1&b=2".to_string()), "entities decoded");
        assert_eq!(base_href(html).as_deref(), Some("https://cdn.example/files/"));
    }

    #[test]
    fn does_not_match_attributes_that_merely_end_in_href() {
        let (links, _) = extract_links(r#"<a xhref="no.zip" href="yes.zip">"#);
        assert!(links.contains(&"yes.zip".to_string()));
        assert!(!links.contains(&"no.zip".to_string()), "xhref is a different attribute");
    }

    #[test]
    fn detects_a_page_whose_links_are_built_by_script() {
        let (links, script_only) = extract_links("<html><script>document.write('<a href=x.zip>')</script></html>");
        assert!(links.is_empty());
        assert!(script_only, "should report that the links are not in the markup");
    }

    #[test]
    fn entity_decoding() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&#38;&#x26;"), "&&");
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
        assert_eq!(decode_entities("100% &pound; free"), "100% &pound; free");
    }

    #[test]
    fn url_list_round_trip() {
        let text = "# my list\nhttps://a.example/1.zip\n\n  https://b.example/2.iso  \n#nope\nnot a url\n";
        let urls = parse_url_list(text);
        assert_eq!(urls, vec!["https://a.example/1.zip", "https://b.example/2.iso"]);
        let exported = url_list_text(&urls);
        assert_eq!(parse_url_list(&exported), urls);
    }

    #[test]
    fn filters_by_type_and_host() {
        let opts = Options { types: vec!["zip".into()], ..Default::default() };
        let z = Url::parse("https://a.example/x.zip").unwrap();
        let i = Url::parse("https://a.example/x.iso").unwrap();
        assert!(looks_like_file(&z, &opts));
        assert!(!looks_like_file(&i, &opts));

        let same = Options { same_host_only: true, ..Default::default() };
        let other = Url::parse("https://b.example/x.zip").unwrap();
        assert!(keep(&z, &same, "a.example"));
        assert!(!keep(&other, &same, "a.example"));

        let inc = Options { include: vec!["/pub/".into()], ..Default::default() };
        assert!(keep(&Url::parse("https://a.example/pub/x.zip").unwrap(), &inc, "a.example"));
        assert!(!keep(&Url::parse("https://a.example/priv/x.zip").unwrap(), &inc, "a.example"));
        // One of them is enough, as the option says.
        let either = Options { include: vec!["/pub/".into(), "/mirror/".into()], ..Default::default() };
        assert!(keep(&Url::parse("https://a.example/pub/x.zip").unwrap(), &either, "a.example"));
        assert!(keep(&Url::parse("https://a.example/mirror/x.zip").unwrap(), &either, "a.example"));
        assert!(!keep(&Url::parse("https://a.example/priv/x.zip").unwrap(), &either, "a.example"));
    }

    #[test]
    fn scripts_styles_and_comments_are_blanked_wherever_they_are() {
        let html = "<a href=a.zip><script>x='<a href=s.zip>'</script><!-- <a href=c.zip> --><style>.x{}</style><a href=b.zip><script>y</script>";
        let clean = blank_non_markup(html);
        assert_eq!(clean.len(), html.len(), "offsets are kept");
        let (links, _) = extract_links(html);
        assert_eq!(links, ["a.zip", "b.zip"]);
        // Long pages stay linear: many regions, each searched for once.
        let big = "<p><script>1</script><a href=f.zip>".repeat(20_000);
        let t = std::time::Instant::now();
        assert_eq!(extract_links(&big).0.len(), 20_000);
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[tokio::test]
    async fn an_endless_page_is_not_read_into_memory() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut req = [0u8; 2048];
            let _ = s.read(&mut req).await;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\n\r\n").await;
            let chunk = format!("{:x}\r\n{}\r\n", 65536, "a".repeat(65536));
            // 64 MiB and more, until the reader hangs up.
            for _ in 0..1024 {
                if s.write_all(chunk.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        let e = fetch(&reqwest::Client::new(), &Url::parse(&format!("http://{addr}/")).unwrap()).await.err().expect("refused");
        assert!(format!("{e:#}").contains("larger than 8 MiB"), "{e:#}");
    }

    #[test]
    fn fragments_are_stripped() {
        let u = strip_fragment(Url::parse("https://a.example/x.zip#part2").unwrap());
        assert_eq!(u.as_str(), "https://a.example/x.zip");
    }
}
