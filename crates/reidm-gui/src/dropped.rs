//! What to do with something dropped on the window, and which copied text is a download link.
//!
//! Dropping: a `.torrent` file opens *Add torrent*; a `.txt` or `.list` list is imported; a
//! link — or a shortcut file holding one (`.url` on Windows, `.webloc` on macOS, a `.desktop`
//! link on Linux) — opens *Add URL* with it filled in. The window system hands over files only,
//! never bare text, so a link dragged straight out of a browser arrives as such a shortcut, or
//! not at all.
//!
//! Copying: *Watch the clipboard* offers a copied link when it points at a file of a type
//! reIDM catches from the browser, or is a magnet link — not every copied web address.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    Urls(Vec<String>),
    Torrent(PathBuf),
    Import(PathBuf),
    /// Nothing reIDM can add: its name, to say so.
    Unsupported(String),
}

/// A link a download can start from.
fn is_link(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ["http://", "https://", "ftp://", "magnet:?"].iter().any(|p| l.starts_with(p))
}

fn extension(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The link inside a shortcut file: `URL=` in a Windows `.url` or a Linux `.desktop` link, the
/// `<string>` after `<key>URL</key>` in a macOS `.webloc`.
pub fn shortcut_link(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("URL=") {
            return Some(v.trim().to_string()).filter(|u| is_link(u));
        }
    }
    let after = text.split("<key>URL</key>").nth(1)?;
    let v = after.split("<string>").nth(1)?.split("</string>").next()?;
    let v = v.trim().replace("&amp;", "&");
    is_link(&v).then_some(v)
}

/// What each dropped item is. `read` reads a small file (for shortcuts); each item is the path
/// the window system gave, or for a platform that sends a link as a "path", that link.
pub fn classify(items: &[PathBuf], read: impl Fn(&Path) -> Option<String>) -> Vec<Dropped> {
    let mut urls = Vec::new();
    let mut out = Vec::new();
    for p in items {
        let s = p.to_string_lossy();
        if is_link(&s) {
            urls.push(s.into_owned());
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.into_owned());
        match extension(&name).as_str() {
            "torrent" => out.push(Dropped::Torrent(p.clone())),
            "txt" | "list" => out.push(Dropped::Import(p.clone())),
            "url" | "webloc" | "desktop" => match read(p).as_deref().and_then(shortcut_link) {
                Some(u) => urls.push(u),
                None => out.push(Dropped::Unsupported(name)),
            },
            _ => out.push(Dropped::Unsupported(name)),
        }
    }
    if !urls.is_empty() {
        out.insert(0, Dropped::Urls(urls));
    }
    out
}

/// A copied text that is worth offering as a download: one link, to a file of a type reIDM
/// catches from the browser, or a magnet link.
pub fn download_link(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 8192 || t.contains(char::is_whitespace) {
        return None;
    }
    let l = t.to_ascii_lowercase();
    if l.starts_with("magnet:?") && l.contains("xt=urn:") {
        return Some(t.to_string());
    }
    if !(l.starts_with("http://") || l.starts_with("https://") || l.starts_with("ftp://")) {
        return None;
    }
    // The path's last part, without the query or fragment.
    let rest = &l[l.find("://")? + 3..];
    let path = rest.split(['?', '#']).next()?;
    let last = path.split('/').skip(1).last().unwrap_or("");
    let ext = extension(last);
    (!ext.is_empty() && reidm_core::grabber::COMMON_TYPES.contains(&ext.as_str())).then(|| t.to_string())
}

/// What *Watch the clipboard* has seen this session: each link is offered once, and nothing the
/// window copied itself is offered at all.
#[derive(Default)]
pub struct ClipboardSeen {
    /// The clipboard's text when last looked at: the same text again is no news.
    last: Option<String>,
    offered: HashSet<String>,
    own: HashSet<String>,
}

impl ClipboardSeen {
    /// What was on the clipboard before watching started isn't news either.
    pub fn prime(&mut self, text: Option<String>) {
        self.last = text;
    }

    /// The window put this on the clipboard (Copy address, Copy path).
    pub fn copied_here(&mut self, text: &str) {
        self.own.insert(text.trim().to_string());
    }

    /// The clipboard now holds `text`: the link to offer, if any.
    pub fn offer(&mut self, text: &str) -> Option<String> {
        if self.last.as_deref() == Some(text) {
            return None;
        }
        self.last = Some(text.to_string());
        if self.own.contains(text.trim()) {
            return None;
        }
        let link = download_link(text)?;
        self.offered.insert(link.clone()).then_some(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_are_sorted_by_what_they_are() {
        let files: Vec<PathBuf> = ["/tmp/debian.torrent", "/tmp/links.TXT", "/tmp/Ubuntu.url", "/tmp/photo.jpg", "https://example.com/a.zip"].iter().map(PathBuf::from).collect();
        let read = |p: &Path| (p.extension()? == "url").then(|| "[InternetShortcut]\r\nURL=https://example.com/ubuntu.iso\r\n".to_string());
        assert_eq!(
            classify(&files, read),
            vec![
                Dropped::Urls(vec!["https://example.com/ubuntu.iso".into(), "https://example.com/a.zip".into()]),
                Dropped::Torrent("/tmp/debian.torrent".into()),
                Dropped::Import("/tmp/links.TXT".into()),
                Dropped::Unsupported("photo.jpg".into()),
            ]
        );
        assert_eq!(classify(&[PathBuf::from("/tmp/broken.url")], |_| Some("nothing here".into())), vec![Dropped::Unsupported("broken.url".into())]);
    }

    #[test]
    fn shortcuts_of_every_system() {
        assert_eq!(shortcut_link("[Desktop Entry]\nType=Link\nURL=https://example.com/f.zip\n").as_deref(), Some("https://example.com/f.zip"));
        let webloc = "<?xml version=\"1.0\"?><plist><dict><key>URL</key><string>https://example.com/f.zip?a=1&amp;b=2</string></dict></plist>";
        assert_eq!(shortcut_link(webloc).as_deref(), Some("https://example.com/f.zip?a=1&b=2"));
        assert_eq!(shortcut_link("URL=javascript:alert(1)"), None, "only links a download can start from");
    }

    #[test]
    fn only_links_to_files_are_offered() {
        assert_eq!(download_link(" https://example.com/pub/ubuntu.ISO?token=x#frag \n").as_deref(), Some("https://example.com/pub/ubuntu.ISO?token=x#frag"));
        assert!(download_link("magnet:?xt=urn:btih:abcdef&dn=x").is_some());
        assert_eq!(download_link("https://example.com/"), None, "a page");
        assert_eq!(download_link("https://example.com/about"), None);
        assert_eq!(download_link("https://example.com.zip"), None, "a host, not a file");
        assert_eq!(download_link("see https://example.com/a.zip"), None, "a sentence");
        assert_eq!(download_link("/home/me/a.zip"), None);
    }

    #[test]
    fn each_link_is_offered_once_and_never_our_own() {
        let mut c = ClipboardSeen::default();
        c.prime(Some("https://example.com/old.zip".into()));
        assert_eq!(c.offer("https://example.com/old.zip"), None, "already there when watching began");
        assert_eq!(c.offer("https://example.com/a.zip").as_deref(), Some("https://example.com/a.zip"));
        assert_eq!(c.offer("https://example.com/a.zip"), None, "unchanged");
        c.offer("something else");
        assert_eq!(c.offer("https://example.com/a.zip"), None, "copied again: not offered again this session");
        c.copied_here("https://example.com/mine.iso");
        assert_eq!(c.offer("https://example.com/mine.iso"), None, "copied by the window itself");
    }
}
