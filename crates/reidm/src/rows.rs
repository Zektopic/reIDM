//! The download list as the window draws it: one small row per download, and only the rows that
//! changed since the caller last asked.
//!
//! `GET /api/downloads` answers with every field of every download, segment maps and torrent
//! file lists included. With a long history that is megabytes, sent several times a second,
//! although the window shows a handful of fields and only a few downloads change at a time. A
//! row leaves out what only the Properties window needs (the segments, a torrent's files, the
//! server's validators, the user agent), and the cache below remembers what each caller was last
//! sent, so a poll costs nothing when nothing changed and a few hundred bytes when one download
//! moved.
//!
//! The cache holds each row's serialised JSON. Whether a row changed is decided by comparing
//! bytes, so it needs no co-operation from the engine beyond a revision that says when looking
//! again is worthwhile.

use reidm_core::{Item, Media, Status};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// How many removals are remembered for callers that are behind. A caller further behind than
/// this gets the whole list again, which is always correct, just bigger.
const REMOVED_KEPT: usize = 4096;

#[derive(Serialize)]
pub struct Row<'a> {
    pub id: u64,
    pub url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referer: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<&'a Media>,
    pub path: &'a std::path::Path,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: &'a Status,
    pub added: u64,
    pub last_try: Option<u64>,
    pub finished_at: Option<u64>,
    pub resumable: bool,
    pub speed_limit: Option<u64>,
    pub scheduled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'a str>,
    /// Connections transferring right now, for the "Downloading (8)" status.
    pub connections: usize,
    /// How many byte ranges the download is split into.
    pub parts: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentRow<'a>>,
}

/// A torrent's summary, without its file list (which can run to tens of thousands of entries).
#[derive(Serialize)]
pub struct TorrentRow<'a> {
    pub info_hash: &'a str,
    pub name: &'a str,
    pub uploaded: u64,
    pub peers: u32,
    pub seeding: bool,
    pub files: usize,
    /// Files left out of the download.
    pub skipped: usize,
}

impl<'a> Row<'a> {
    pub fn of(it: &'a Item) -> Self {
        Row {
            id: it.id,
            url: &it.request.url,
            referer: it.request.referer.as_deref(),
            media: it.request.media.as_ref(),
            path: &it.path,
            size: it.size,
            downloaded: it.downloaded,
            status: &it.status,
            added: it.added,
            last_try: it.last_try,
            finished_at: it.finished_at,
            resumable: it.resumable(),
            speed_limit: it.speed_limit,
            scheduled: it.scheduled,
            note: it.note.as_deref(),
            connections: it.segments().iter().filter(|s| s.active).count(),
            parts: it.segments().len(),
            torrent: it.torrent.as_ref().map(|t| TorrentRow {
                info_hash: &t.info_hash,
                name: &t.name,
                uploaded: t.uploaded,
                peers: t.peers,
                seeding: t.seeding,
                files: t.files.len(),
                skipped: t.files.iter().filter(|f| !f.selected).count(),
            }),
        }
    }
}

struct Entry {
    json: Box<[u8]>,
    /// The sequence number of the rebuild that last changed this row.
    changed: u64,
}

/// What a request for rows gets back.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// The caller is up to date.
    NotModified,
    /// A JSON body: `{"cursor", "full", "rows", "removed"}`.
    Body(Vec<u8>),
}

pub struct RowCache {
    /// Differs between runs of the service, so a cursor from a previous run is never mistaken
    /// for one of this run's: sequence numbers start again at every start.
    boot: u64,
    /// The engine revision the rows were last built at.
    rev: Option<u64>,
    seq: u64,
    rows: HashMap<u64, Entry>,
    /// `(seq, id)` of recent removals, oldest first.
    removed: VecDeque<(u64, u64)>,
    /// Removals at or before this sequence number have been forgotten, so a caller whose cursor
    /// is older than it cannot be given a correct delta.
    forgotten: u64,
}

impl RowCache {
    pub fn new(boot: u64) -> Self {
        RowCache { boot, rev: None, seq: 0, rows: HashMap::new(), removed: VecDeque::new(), forgotten: 0 }
    }

    /// Whether the rows were built at this revision, so `refresh` would find nothing new.
    pub fn is_current(&self, rev: u64) -> bool {
        self.rev == Some(rev)
    }

    /// Bring the rows up to date with `items`, the list at revision `rev`.
    pub fn refresh(&mut self, rev: u64, items: &[Item]) {
        let next = self.seq + 1;
        let mut changed = false;
        let mut seen = std::collections::HashSet::with_capacity(items.len());
        for it in items {
            seen.insert(it.id);
            let json = serde_json::to_vec(&Row::of(it)).unwrap_or_default();
            match self.rows.get_mut(&it.id) {
                Some(e) if *e.json == *json => {}
                Some(e) => {
                    e.json = json.into_boxed_slice();
                    e.changed = next;
                    changed = true;
                }
                None => {
                    self.rows.insert(it.id, Entry { json: json.into_boxed_slice(), changed: next });
                    changed = true;
                }
            }
        }
        if self.rows.len() > seen.len() {
            let gone: Vec<u64> = self.rows.keys().copied().filter(|id| !seen.contains(id)).collect();
            for id in gone {
                self.rows.remove(&id);
                self.removed.push_back((next, id));
                changed = true;
            }
            while self.removed.len() > REMOVED_KEPT {
                if let Some((s, _)) = self.removed.pop_front() {
                    self.forgotten = s;
                }
            }
        }
        if changed {
            self.seq = next;
        }
        self.rev = Some(rev);
    }

    /// The cursor a caller holds once it has applied the current rows; also the `ETag`.
    pub fn cursor(&self) -> String {
        format!("{:x}-{}", self.boot, self.seq)
    }

    /// The rows a caller holding `since` is missing: none, only the changed ones, or (for a
    /// first request, a cursor from another run, or one too old) all of them.
    pub fn answer(&self, since: Option<&str>) -> Answer {
        let since = since.and_then(|s| {
            let (boot, seq) = s.trim().trim_matches('"').split_once('-')?;
            (u64::from_str_radix(boot, 16).ok()? == self.boot).then_some(seq.parse::<u64>().ok()?)
        });
        let since = since.filter(|s| *s <= self.seq && *s >= self.forgotten);
        if since == Some(self.seq) {
            return Answer::NotModified;
        }
        let full = since.is_none();
        let after = since.unwrap_or(0);
        let mut ids: Vec<u64> = self.rows.iter().filter(|(_, e)| full || e.changed > after).map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let removed: Vec<u64> = if full { Vec::new() } else { self.removed.iter().filter(|(s, _)| *s > after).map(|(_, id)| *id).collect() };

        let size: usize = ids.iter().map(|id| self.rows[id].json.len() + 1).sum();
        let mut out = Vec::with_capacity(size + removed.len() * 8 + 64);
        out.extend_from_slice(format!(r#"{{"cursor":"{}","full":{full},"rows":["#, self.cursor()).as_bytes());
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            out.extend_from_slice(&self.rows[id].json);
        }
        out.extend_from_slice(br#"],"removed":"#);
        out.extend_from_slice(&serde_json::to_vec(&removed).unwrap_or_default());
        out.push(b'}');
        Answer::Body(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A download as the engine keeps one: eight connections' ranges, a browser's user agent.
    fn item(id: u64, downloaded: u64) -> Item {
        let segments: Vec<serde_json::Value> = (0..8u64).map(|k| serde_json::json!({ "start": k * 125, "end": k * 125 + 124, "done": downloaded.min(125), "finished": false, "active": false })).collect();
        serde_json::from_value(serde_json::json!({
            "id": id,
            "request": { "url": format!("https://example.com/{id}.bin"), "referer": null, "cookie": "secret=1", "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36", "filename": null, "dir": null },
            "path": format!("/tmp/{id}.bin"),
            "size": 1000,
            "downloaded": downloaded,
            "status": { "state": "Paused" },
            "segments": segments,
            "resumable": true,
            "etag": "\"abc\"",
        }))
        .unwrap()
    }

    fn body(a: Answer) -> serde_json::Value {
        match a {
            Answer::Body(b) => serde_json::from_slice(&b).unwrap(),
            Answer::NotModified => panic!("expected a body"),
        }
    }

    #[test]
    fn a_row_leaves_out_what_the_list_does_not_show() {
        let it = item(1, 10);
        let v = serde_json::to_value(Row::of(&it)).unwrap();
        assert_eq!(v["url"], "https://example.com/1.bin");
        assert_eq!(v["parts"], 8);
        assert_eq!(v["connections"], 0);
        assert_eq!(v["status"]["state"], "Paused");
        for absent in ["segments", "cookie", "user_agent", "etag", "request"] {
            assert!(v.get(absent).is_none(), "{absent} should not be in a row");
        }
    }

    #[test]
    fn first_request_gets_everything_then_only_changes() {
        let mut c = RowCache::new(0xab);
        c.refresh(1, &[item(1, 0), item(2, 0), item(3, 0)]);
        let first = body(c.answer(None));
        assert_eq!(first["full"], true);
        assert_eq!(first["rows"].as_array().unwrap().len(), 3);
        let cur = first["cursor"].as_str().unwrap().to_string();

        // Nothing moved: the caller is told so, with no body at all.
        c.refresh(2, &[item(1, 0), item(2, 0), item(3, 0)]);
        assert_eq!(c.answer(Some(&cur)), Answer::NotModified, "an unchanged list must cost nothing");

        // One download moved and one went away.
        c.refresh(3, &[item(1, 0), item(2, 50)]);
        let d = body(c.answer(Some(&cur)));
        assert_eq!(d["full"], false);
        let rows = d["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], 2);
        assert_eq!(rows[0]["downloaded"], 50);
        assert_eq!(d["removed"], serde_json::json!([3]));

        // The ETag form of the cursor (quoted) is accepted too.
        let now = d["cursor"].as_str().unwrap();
        assert_eq!(c.answer(Some(&format!("\"{now}\""))), Answer::NotModified);
    }

    #[test]
    fn a_cursor_from_another_run_or_too_old_gets_the_whole_list() {
        let mut c = RowCache::new(1);
        c.refresh(1, &[item(1, 0)]);
        assert_eq!(body(c.answer(Some("2-1")))["full"], true, "another run's cursor");
        assert_eq!(body(c.answer(Some("1-99")))["full"], true, "a cursor from the future");
        assert_eq!(body(c.answer(Some("garbage")))["full"], true);

        // More removals than are remembered: a caller from before them can't get a delta.
        let many: Vec<Item> = (10..10 + REMOVED_KEPT as u64 + 10).map(|i| item(i, 0)).collect();
        c.refresh(2, &many);
        let cur = c.cursor();
        c.refresh(3, &[]);
        assert_eq!(body(c.answer(Some(&cur)))["full"], true, "removals it never heard of can't be sent as a delta");
        assert_eq!(c.answer(Some(&c.cursor())), Answer::NotModified);
    }

    #[test]
    fn fifty_thousand_rows_stay_small() {
        let items: Vec<Item> = (1..=50_000).map(|i| item(i, i)).collect();
        let mut c = RowCache::new(7);
        c.refresh(1, &items);
        let Answer::Body(b) = c.answer(None) else { panic!() };
        assert!(b.len() < 50_000 * 300, "{} bytes for 50k rows", b.len());
        let full = serde_json::to_vec(&items).unwrap();
        assert!(b.len() * 3 < full.len(), "rows {} vs items {}", b.len(), full.len());
        let cur = c.cursor();
        let mut moved = items.clone();
        moved[123].downloaded += 1;
        c.refresh(2, &moved);
        let Answer::Body(d) = c.answer(Some(&cur)) else { panic!() };
        assert!(d.len() < 1024, "a one-row delta is {} bytes", d.len());
    }
}
