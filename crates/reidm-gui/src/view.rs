//! What the list shows, worked out once per change rather than once per frame.
//!
//! Filtering, sorting and counting tens of thousands of rows takes milliseconds; the window may
//! draw many frames between two changes to the list (a mouse move, a menu opening). So the
//! visible order and every count are kept here, and rebuilt only when the list, the chosen
//! view, the search text or the sort order change.

use crate::api::Snapshot;
use crate::model::{Status, View, CATEGORIES};
use std::cmp::Ordering;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SortKey {
    Name,
    Size,
    /// Running, then waiting, paused, failed and finished.
    Status,
    /// Time left: the soonest to finish first; downloads with no estimate last.
    TimeLeft,
    Rate,
    LastTry,
    Address,
    /// Place in the queue: unfinished downloads first, in the order they will start.
    Queue,
}

#[derive(Clone, PartialEq)]
struct Key {
    rev: u64,
    view: View,
    search: String,
    sort: SortKey,
    desc: bool,
}

/// Counts for the status bar and the category tree.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Counts {
    pub total: usize,
    pub running: usize,
    pub done: usize,
    pub failed: usize,
    /// In the scheduled queue and not finished.
    pub queued: usize,
    /// One per entry of `CATEGORIES` (All first).
    pub categories: [usize; 8],
    pub unfinished: usize,
    /// Waiting for a free slot (`Queued`).
    pub waiting: usize,
    pub paused: usize,
    /// The service keeps a queue order (rows carry `order`).
    pub ordered: bool,
    /// The service checks checksums (rows carry `checksum_ok`).
    pub checksums: bool,
}

impl Counts {
    pub fn of(&self, v: View) -> usize {
        match v {
            View::Category(c) => CATEGORIES.iter().position(|x| *x == c).map(|i| self.categories[i]).unwrap_or(0),
            View::Unfinished => self.unfinished,
            View::Finished => self.done,
            View::Queue => self.queued,
        }
    }
}

#[derive(Default)]
pub struct ViewModel {
    key: Option<Key>,
    /// Positions in the snapshot's `items` of the visible rows, in display order.
    pub rows: Vec<usize>,
    pub counts: Counts,
    /// The unfinished downloads in queue order, and each one's place (from 1).
    pub queue: Vec<u64>,
    pub places: std::collections::HashMap<u64, usize>,
    /// How many times the rows were rebuilt (for tests and the debug overlay).
    pub builds: u64,
}

impl ViewModel {
    /// Bring the rows up to date. Returns whether anything was rebuilt.
    pub fn update(&mut self, snap: &Snapshot, view: View, search: &str, sort: SortKey, desc: bool) -> bool {
        let key = Key { rev: snap.rev, view, search: search.trim().to_lowercase(), sort, desc };
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        let items = &snap.items;
        let mut counts = Counts { total: items.len(), ..Default::default() };
        let mut rows = Vec::new();
        for (i, it) in items.iter().enumerate() {
            match it.status {
                Status::Running => counts.running += 1,
                Status::Done => counts.done += 1,
                Status::Failed(_) => counts.failed += 1,
                Status::Queued => counts.waiting += 1,
                Status::Paused => counts.paused += 1,
            }
            if it.status != Status::Done {
                counts.unfinished += 1;
                if it.scheduled {
                    counts.queued += 1;
                }
            }
            counts.ordered |= it.order.is_some();
            counts.checksums |= it.checksum_ok.is_some();
            counts.categories[0] += 1;
            if let Some(c) = CATEGORIES.iter().position(|c| *c == it.category()) {
                counts.categories[c] += 1;
            }
            if view.matches(it) && (key.search.is_empty() || it.name_lower().contains(&key.search) || it.url_lower().contains(&key.search)) {
                rows.push(i);
            }
        }
        // Only rebuilt with the list itself: a new search or sort doesn't change the queue.
        if self.key.as_ref().is_none_or(|k| k.rev != key.rev) {
            self.queue = crate::queue::queue(items.iter().map(|a| a.as_ref()));
            self.places = crate::queue::places(&self.queue);
        }
        let places = &self.places;
        let place = |i: usize| places.get(&items[i].id).copied().unwrap_or(usize::MAX);
        let rate = |i: usize| snap.rates.get(&items[i].id).copied().unwrap_or(0.0);
        let eta = |i: usize| {
            let it = &items[i];
            match (it.size, rate(i)) {
                (Some(sz), r) if r > 1.0 && sz > it.downloaded && it.status == Status::Running => (sz - it.downloaded) as f64 / r,
                _ => f64::INFINITY,
            }
        };
        rows.sort_by(|&a, &b| {
            let (x, y) = (&items[a], &items[b]);
            let o = match sort {
                SortKey::Name => x.name_lower().cmp(y.name_lower()),
                SortKey::Size => x.size.unwrap_or(0).cmp(&y.size.unwrap_or(0)),
                SortKey::Status => x.status.rank().cmp(&y.status.rank()),
                SortKey::TimeLeft => eta(a).partial_cmp(&eta(b)).unwrap_or(Ordering::Equal),
                SortKey::Rate => rate(a).partial_cmp(&rate(b)).unwrap_or(Ordering::Equal),
                SortKey::LastTry => x.last_try.unwrap_or(0).cmp(&y.last_try.unwrap_or(0)),
                SortKey::Address => x.url_lower().cmp(y.url_lower()),
                SortKey::Queue => place(a).cmp(&place(b)),
            }
            // Equal keys keep a stable order (newest last), so rows don't jump between frames.
            .then(x.id.cmp(&y.id));
            if desc {
                o.reverse()
            } else {
                o
            }
        });
        self.rows = rows;
        self.counts = counts;
        self.key = Some(key);
        self.builds += 1;
        true
    }

    /// The ids of the visible rows, in order.
    pub fn ids<'a>(&'a self, snap: &'a Snapshot) -> impl Iterator<Item = u64> + 'a {
        self.rows.iter().map(|&i| snap.items[i].id)
    }
}

/// A move of the keyboard focus through the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// Rows a page moves by.
const PAGE: usize = 10;

/// The row the focus moves to among `len` rows, from row `from` (none yet: the first row, or
/// the last for End).
pub fn step(len: usize, from: Option<usize>, nav: Nav) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (from, nav) {
        (_, Nav::Home) => 0,
        (_, Nav::End) => last,
        (None, _) => 0,
        (Some(i), Nav::Up) => i.saturating_sub(1),
        (Some(i), Nav::PageUp) => i.saturating_sub(PAGE),
        (Some(i), Nav::Down) => (i + 1).min(last),
        (Some(i), Nav::PageDown) => (i + PAGE).min(last),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Category, Item};
    use std::sync::Arc;

    pub fn row(id: u64, name: &str, state: &str, downloaded: u64) -> Item {
        serde_json::from_value(serde_json::json!({
            "id": id, "url": format!("https://example.com/{name}"), "path": format!("/dl/{name}"),
            "size": 1000, "downloaded": downloaded, "status": if state == "Failed" { serde_json::json!({ "state": state, "error": "HTTP 404" }) } else { serde_json::json!({ "state": state }) },
            "connections": 0, "parts": 1,
        }))
        .unwrap()
    }

    pub fn snap(items: Vec<Item>, rev: u64) -> Snapshot {
        let items: Vec<Arc<Item>> = items.into_iter().map(Arc::new).collect();
        let index = items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
        Snapshot { rev, items: Arc::new(items), index: Arc::new(index), ..Default::default() }
    }

    #[test]
    fn rows_are_rebuilt_only_when_something_they_depend_on_changed() {
        let s = snap(vec![row(1, "b.zip", "Done", 1000), row(2, "a.mp4", "Running", 10), row(3, "c.iso", "Paused", 0)], 1);
        let mut vm = ViewModel::default();
        let all = View::Category(Category::All);
        assert!(vm.update(&s, all, "", SortKey::Name, false));
        for _ in 0..100 {
            assert!(!vm.update(&s, all, "", SortKey::Name, false), "an unchanged frame must not rebuild");
        }
        assert_eq!(vm.builds, 1);
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![2, 1, 3]);
        assert!(vm.update(&s, all, "", SortKey::Name, true));
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![3, 1, 2]);
        assert!(vm.update(&s, all, " A.MP4 ", SortKey::Name, true), "search is case-insensitive and trimmed");
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![2]);
        let s2 = snap(s.items.iter().map(|i| (**i).clone()).collect(), 2);
        assert!(vm.update(&s2, all, " A.MP4 ", SortKey::Name, true), "a new snapshot rebuilds");
    }

    #[test]
    fn the_keyboard_walks_the_list_and_stops_at_its_ends() {
        assert_eq!(step(0, None, Nav::Down), None, "nothing to move to");
        assert_eq!(step(5, None, Nav::Down), Some(0));
        assert_eq!(step(5, None, Nav::End), Some(4));
        assert_eq!(step(5, Some(4), Nav::Down), Some(4));
        assert_eq!(step(5, Some(0), Nav::Up), Some(0));
        assert_eq!(step(30, Some(3), Nav::PageDown), Some(13));
        assert_eq!(step(30, Some(25), Nav::PageDown), Some(29));
        assert_eq!(step(30, Some(3), Nav::PageUp), Some(0));
    }

    #[test]
    fn counts_and_status_order() {
        let mut q = row(4, "q.zip", "Paused", 0);
        q.scheduled = true;
        let s = snap(vec![row(1, "b.zip", "Done", 1000), row(2, "a.mp4", "Running", 10), row(3, "c.iso", "Failed", 0), q], 1);
        let mut vm = ViewModel::default();
        vm.update(&s, View::Category(Category::All), "", SortKey::Status, false);
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![2, 4, 3, 1], "running, paused, failed, done");
        let c = &vm.counts;
        assert_eq!((c.total, c.running, c.done, c.failed, c.queued, c.unfinished), (4, 1, 1, 1, 1, 3));
        assert_eq!(c.of(View::Category(Category::Video)), 1);
        assert_eq!(c.of(View::Category(Category::Compressed)), 2);
        assert_eq!(c.of(View::Finished), 1);
        vm.update(&s, View::Queue, "", SortKey::Name, false);
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![4]);
    }

    #[test]
    fn time_left_sorts_by_what_remains_not_by_rate() {
        let mut s = snap(vec![row(1, "near.bin", "Running", 900), row(2, "far.bin", "Running", 0), row(3, "idle.bin", "Paused", 0)], 1);
        // The far one is faster, but has much more left.
        s.rates = Arc::new([(1, 10.0), (2, 50.0)].into_iter().collect());
        let mut vm = ViewModel::default();
        vm.update(&s, View::Category(Category::All), "", SortKey::TimeLeft, false);
        assert_eq!(vm.ids(&s).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn fifty_thousand_rows_build_quickly() {
        let items: Vec<Item> = (1..=50_000).map(|i| row(i, &format!("file-{}.zip", (i * 7919) % 50_000), if i % 50 == 0 { "Running" } else { "Done" }, i)).collect();
        let s = snap(items, 1);
        let mut vm = ViewModel::default();
        let t = std::time::Instant::now();
        vm.update(&s, View::Category(Category::All), "", SortKey::Name, false);
        let built = t.elapsed();
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            vm.update(&s, View::Category(Category::All), "", SortKey::Name, false);
        }
        eprintln!("50k rows: build {built:?}, 1000 unchanged frames {:?}", t.elapsed());
        assert_eq!(vm.rows.len(), 50_000);
        assert!(t.elapsed() < std::time::Duration::from_millis(50), "an unchanged frame costs nothing");
    }
}
