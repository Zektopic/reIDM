//! The download list in memory, and how it reaches the disk.
//!
//! Three things shape this module:
//!
//! - **Nothing slow happens under the lock.** Every API call and every running download takes
//!   the list's lock, many times a second. Serialising thousands of items and writing them to a
//!   disk under it stopped the whole service for as long as the disk took. Items are therefore
//!   held copy-on-write ([`Shared`]): a save takes the lock only long enough to copy pointers,
//!   then serialises and writes on its own thread.
//! - **Changes are noticed, not announced.** Whoever changes the list through [`DbGuard`]
//!   marks it changed by doing so: the revision moves on and a save is scheduled. Code that
//!   forgets to ask for a save cannot lose a change, and a burst of changes (a thousand links,
//!   a download's progress) becomes one write.
//! - **A crash never costs the list.** A save is written to a temporary file, flushed to the
//!   disk, and renamed over the old one; the old one is kept as `.bak`. Reading falls back to
//!   the backup when the main file is damaged, and refuses to start with an empty list when the
//!   file exists but cannot be read — saving an empty list over it would destroy it.

use crate::{url_key_of, Item};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LockResult, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How long a change may wait for company before it is written. Progress changes many times a
/// second; writing each would rewrite the list continuously.
const DEBOUNCE: Duration = Duration::from_secs(2);
/// After a failed save (a full disk, say), how long to wait before trying again.
const RETRY: Duration = Duration::from_secs(5);

/// A value shared copy-on-write: cloning it is a pointer copy, and changing it copies the value
/// only while a save is still holding the old one.
#[derive(Debug, Clone)]
pub(crate) struct Shared<T>(Arc<T>);

impl<T: Clone> Shared<T> {
    pub(crate) fn new(v: T) -> Self {
        Shared(Arc::new(v))
    }

    pub(crate) fn into_inner(self) -> T {
        Arc::try_unwrap(self.0).unwrap_or_else(|a| (*a).clone())
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        T::deserialize(d).map(|v| Shared(Arc::new(v)))
    }
}

/// The list. Its shape on disk is the one every version has written: `next_id` and `items`.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Db {
    pub(crate) next_id: u64,
    /// Change an item's address only through [`Db::set_url`], which keeps the index right.
    pub(crate) items: BTreeMap<u64, Shared<Item>>,
    /// Which items have each address (by [`crate::url_key`]), so recognising a link already in
    /// the list does not parse every address in it. Several items can share an address: the
    /// same video page in two formats.
    #[serde(skip)]
    by_key: HashMap<String, Vec<u64>>,
}

impl Db {
    pub(crate) fn fresh() -> Self {
        Db { next_id: 1, ..Default::default() }
    }

    /// Build the address index; called once after loading.
    pub(crate) fn reindex(&mut self) {
        self.by_key.clear();
        for (id, it) in &self.items {
            if let Some(k) = url_key_of(&it.request.url) {
                self.by_key.entry(k).or_default().push(*id);
            }
        }
    }

    pub(crate) fn insert(&mut self, it: Item) {
        if let Some(k) = url_key_of(&it.request.url) {
            self.by_key.entry(k).or_default().push(it.id);
        }
        self.items.insert(it.id, Shared::new(it));
    }

    pub(crate) fn remove(&mut self, id: u64) -> Option<Item> {
        let it = self.items.remove(&id)?.into_inner();
        if let Some(k) = url_key_of(&it.request.url) {
            self.unindex(&k, id);
        }
        Some(it)
    }

    pub(crate) fn set_url(&mut self, id: u64, url: String) {
        let Some(it) = self.items.get_mut(&id) else { return };
        let old = url_key_of(&it.request.url);
        let new = url_key_of(&url);
        it.request.url = url;
        if old != new {
            if let Some(k) = old {
                self.unindex(&k, id);
            }
            if let Some(k) = new {
                self.by_key.entry(k).or_default().push(id);
            }
        }
    }

    fn unindex(&mut self, key: &str, id: u64) {
        if let Some(ids) = self.by_key.get_mut(key) {
            ids.retain(|x| *x != id);
            if ids.is_empty() {
                self.by_key.remove(key);
            }
        }
    }

    /// The items whose address has this key.
    pub(crate) fn with_key<'a>(&'a self, key: &str) -> impl Iterator<Item = &'a Item> + 'a {
        self.by_key.get(key).into_iter().flatten().filter_map(|id| self.items.get(id)).map(|s| &**s)
    }
}

/// What a save writes: the list without its index.
#[derive(Serialize)]
struct Snapshot {
    next_id: u64,
    items: BTreeMap<u64, Shared<Item>>,
}

/// The list behind its lock, with the revision counter and the saver that changes to it drive.
pub(crate) struct DbCell {
    db: Mutex<Db>,
    rev: Arc<AtomicU64>,
    saver: Saver,
    path: PathBuf,
}

/// The lock on the list. Changing anything through it (anything that needs `&mut Db`) moves the
/// revision on and schedules a save when the lock is let go.
pub(crate) struct DbGuard<'a> {
    g: MutexGuard<'a, Db>,
    cell: &'a DbCell,
    touched: bool,
}

impl Deref for DbGuard<'_> {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.g
    }
}

impl DerefMut for DbGuard<'_> {
    fn deref_mut(&mut self) -> &mut Db {
        self.touched = true;
        &mut self.g
    }
}

impl Drop for DbGuard<'_> {
    fn drop(&mut self) {
        if self.touched {
            self.cell.rev.fetch_add(1, Ordering::Relaxed);
            self.cell.saver.request(false);
        }
    }
}

impl DbCell {
    /// Hold `db`, saved to `path`, and start the thread that saves it. The thread ends when
    /// [`DbCell::close`] is called.
    pub(crate) fn start(db: Db, path: PathBuf, rev: Arc<AtomicU64>) -> Arc<Self> {
        let cell = Arc::new(DbCell { db: Mutex::new(db), rev, saver: Saver::default(), path });
        let c = cell.clone();
        std::thread::Builder::new()
            .name("reidm-save".into())
            .spawn(move || c.saver_loop())
            .expect("cannot start the thread that saves the download list");
        cell
    }

    /// Shaped like [`Mutex::lock`], so `db.lock().unwrap()` reads as it always has.
    pub(crate) fn lock(&self) -> LockResult<DbGuard<'_>> {
        match self.db.lock() {
            Ok(g) => Ok(DbGuard { g, cell: self, touched: false }),
            Err(p) => Err(PoisonError::new(DbGuard { g: p.into_inner(), cell: self, touched: false })),
        }
    }

    /// Ask for a save: soon, or (`urgent`) at once. Returns the save's number for [`DbCell::wait`].
    pub(crate) fn request_save(&self, urgent: bool) -> u64 {
        self.saver.request(urgent)
    }

    /// Wait until save number `n` (or a later one) has reached the disk. False when it has not
    /// within `limit` — the disk is failing, or very slow.
    pub(crate) fn wait(&self, n: u64, limit: Duration) -> bool {
        self.saver.wait(n, limit)
    }

    /// The last save's error, if the last save failed.
    pub(crate) fn save_error(&self) -> Option<String> {
        self.saver.state.lock().unwrap().error.clone()
    }

    /// Write what is pending and stop the saving thread.
    pub(crate) fn close(&self) {
        let mut s = self.saver.state.lock().unwrap();
        s.closing = true;
        self.saver.cv.notify_all();
    }

    fn write_now(&self) -> Result<()> {
        let snap = {
            let db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
            Snapshot { next_id: db.next_id, items: db.items.clone() }
        };
        // Compact: a list of thousands is several times smaller, and as quick to read back.
        let bytes = serde_json::to_vec(&snap)?;
        drop(snap);
        write_atomic(&self.path, &bytes)
    }

    fn saver_loop(&self) {
        let s = &self.saver;
        loop {
            let target = {
                let mut st = s.state.lock().unwrap();
                loop {
                    if st.wanted <= st.written {
                        if st.closing {
                            return;
                        }
                        st = s.cv.wait(st).unwrap();
                        continue;
                    }
                    let now = Instant::now();
                    let mut due = if st.closing || st.urgent { now } else { st.dirty_since.unwrap_or(now) + DEBOUNCE };
                    if let Some(r) = st.retry_at.filter(|_| !st.closing) {
                        due = due.max(r);
                    }
                    if now >= due {
                        break;
                    }
                    st = s.cv.wait_timeout(st, due - now).unwrap().0;
                }
                st.urgent = false;
                st.dirty_since = None;
                st.wanted
            };
            let res = self.write_now();
            let mut st = s.state.lock().unwrap();
            match res {
                Ok(()) => {
                    st.written = st.written.max(target);
                    if st.error.take().is_some() {
                        tracing::info!("the download list is being saved again");
                    }
                    st.retry_at = None;
                }
                Err(e) => {
                    let msg = format!("could not save the download list to {}: {e:#}", self.path.display());
                    // Once per new error: a full disk would otherwise log every few seconds.
                    if st.error.as_deref() != Some(msg.as_str()) {
                        tracing::error!("{msg}; it will be tried again, and what is in memory is unaffected");
                    }
                    st.error = Some(msg);
                    let now = Instant::now();
                    st.retry_at = Some(now + RETRY);
                    st.dirty_since.get_or_insert(now);
                    if st.closing {
                        // Shutting down: one last attempt has been made, and waiting longer would
                        // only hold the exit up.
                        st.written = st.wanted;
                    }
                }
            }
            s.cv.notify_all();
        }
    }
}

#[derive(Default)]
struct Saver {
    state: Mutex<SaveState>,
    cv: Condvar,
}

#[derive(Default)]
struct SaveState {
    /// Saves asked for, and the number of the last one that is on disk.
    wanted: u64,
    written: u64,
    urgent: bool,
    /// When the oldest unsaved change was made.
    dirty_since: Option<Instant>,
    retry_at: Option<Instant>,
    error: Option<String>,
    closing: bool,
}

impl Saver {
    fn request(&self, urgent: bool) -> u64 {
        let mut s = self.state.lock().unwrap();
        s.wanted += 1;
        s.dirty_since.get_or_insert_with(Instant::now);
        s.urgent |= urgent;
        self.cv.notify_all();
        s.wanted
    }

    fn wait(&self, n: u64, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut s = self.state.lock().unwrap();
        while s.written < n {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            s = self.cv.wait_timeout(s, left).unwrap().0;
        }
        true
    }
}

/// `downloads.json` → `downloads.json<suffix>`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Replace `path` with `bytes` so that, whatever happens part way, `path` afterwards holds either
/// the old contents or the new ones — never a mix, never nothing. The old contents are kept as
/// `<path>.bak`.
///
/// On Unix the file is readable by its owner only: the list holds the browser's cookies for
/// every download.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = sibling(path, ".tmp");
    // A leftover from a crash may have other permissions, which writing into it would keep.
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp).with_context(|| format!("cannot create {}", tmp.display()))?;
    f.write_all(bytes)?;
    // Without this a power cut soon after the rename can leave the new name on an empty file:
    // the rename reaches the disk before the data does on several file systems.
    f.sync_all()?;
    drop(f);
    if path.exists() {
        keep_backup(path);
    }
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    sync_dir(path);
    Ok(())
}

/// Make the current `path` the backup. A hard link costs nothing however large the list is; a
/// file system without them gets a copy.
fn keep_backup(path: &Path) {
    let bak = sibling(path, ".bak");
    let staging = sibling(path, ".bak.tmp");
    let _ = std::fs::remove_file(&staging);
    if std::fs::hard_link(path, &staging).is_err() && std::fs::copy(path, &staging).is_err() {
        return;
    }
    let _ = std::fs::rename(&staging, &bak);
}

/// Make a rename in `path`'s folder durable. Only Unix needs (and allows) this.
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(d) = path.parent() {
        if let Ok(f) = std::fs::File::open(d) {
            let _ = f.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Read a JSON file written by [`write_atomic`]. `Ok(None)` means there is none yet.
///
/// A damaged file is moved aside as `<path>.corrupt-<time>` and its backup used; with no usable
/// backup, it is `Ok(None)` and a warning. A file that exists but cannot be read at all (its
/// permissions, a disk error) is an error unless the backup can stand in: starting empty would
/// save an empty list over the real one.
///
/// Whatever was recovered or lost is described in `warnings`, for the user to be told.
pub(crate) fn load<T: DeserializeOwned>(path: &Path, what: &str, warnings: &mut Vec<String>) -> Result<Option<T>> {
    let bak = sibling(path, ".bak");
    let parse = |b: &[u8]| -> Result<T> {
        if b.iter().all(u8::is_ascii_whitespace) {
            bail!("it is empty");
        }
        Ok(serde_json::from_slice(b)?)
    };
    let mut from_backup = |problem: String| -> Option<T> {
        let restored = std::fs::read(&bak).ok().and_then(|b| parse(&b).ok());
        let msg = match &restored {
            Some(_) => format!("{problem}; the previous copy of the {what} was restored from {}", bak.display()),
            None => format!("{problem}; there is no usable backup, so reIDM starts with an empty {what}"),
        };
        tracing::warn!("{msg}");
        warnings.push(msg);
        restored
    };
    match std::fs::read(path) {
        Ok(b) => match parse(&b) {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                let kept = sibling(path, &format!(".corrupt-{}", crate::now_secs()));
                let problem = match std::fs::rename(path, &kept) {
                    Ok(()) => format!("{} was damaged ({e:#}) and has been kept as {}", path.display(), kept.display()),
                    Err(_) => format!("{} is damaged ({e:#})", path.display()),
                };
                Ok(from_backup(problem))
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if bak.exists() {
                Ok(from_backup(format!("{} was missing", path.display())))
            } else {
                Ok(None)
            }
        }
        Err(e) => {
            let problem = format!("cannot read {}: {e}", path.display());
            match std::fs::read(&bak).ok().and_then(|b| parse(&b).ok()) {
                Some(v) => {
                    let msg = format!("{problem}; using the previous copy from {}", bak.display());
                    tracing::warn!("{msg}");
                    warnings.push(msg);
                    Ok(Some(v))
                }
                None => bail!(
                    "{problem}. reIDM will not start with an empty {what} and save it over that file: check the file's owner and permissions (running reIDM with sudo can leave it owned by root), then start reIDM again"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("reidm-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct V {
        n: u32,
    }

    #[test]
    fn a_save_keeps_the_previous_copy_as_a_backup() {
        let d = dir("bak");
        let p = d.join("x.json");
        write_atomic(&p, br#"{"n":1}"#).unwrap();
        write_atomic(&p, br#"{"n":2}"#).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), br#"{"n":2}"#);
        assert_eq!(std::fs::read(sibling(&p, ".bak")).unwrap(), br#"{"n":1}"#);
        assert!(!sibling(&p, ".tmp").exists());
    }

    #[test]
    fn an_empty_file_after_a_crash_is_recovered_from_the_backup() {
        let d = dir("empty");
        let p = d.join("x.json");
        std::fs::write(sibling(&p, ".bak"), br#"{"n":7}"#).unwrap();
        std::fs::write(&p, b"").unwrap();
        let mut w = Vec::new();
        assert_eq!(load::<V>(&p, "list", &mut w).unwrap(), Some(V { n: 7 }));
        assert_eq!(w.len(), 1, "the user is told: {w:?}");
        // The damaged file is kept, not deleted.
        assert!(std::fs::read_dir(&d).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains(".corrupt-")));
    }

    #[test]
    fn a_damaged_file_without_a_backup_starts_empty_and_says_so() {
        let d = dir("trunc");
        let p = d.join("x.json");
        std::fs::write(&p, br#"{"n":"#).unwrap();
        let mut w = Vec::new();
        assert_eq!(load::<V>(&p, "list", &mut w).unwrap(), None);
        assert!(w[0].contains("no usable backup"), "{w:?}");
        assert!(!p.exists(), "moved aside, so the next save cannot be mistaken for it");
    }

    #[test]
    fn a_missing_file_is_a_first_run_unless_a_backup_exists() {
        let d = dir("missing");
        let p = d.join("x.json");
        let mut w = Vec::new();
        assert_eq!(load::<V>(&p, "list", &mut w).unwrap(), None);
        assert!(w.is_empty());
        std::fs::write(sibling(&p, ".bak"), br#"{"n":3}"#).unwrap();
        assert_eq!(load::<V>(&p, "list", &mut w).unwrap(), Some(V { n: 3 }));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_never_replaced_by_an_empty_list() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("unreadable");
        let p = d.join("x.json");
        std::fs::write(&p, br#"{"n":5}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&p).is_ok() {
            return; // running as root: permissions can't make a file unreadable
        }
        let mut w = Vec::new();
        let e = load::<V>(&p, "list", &mut w).unwrap_err();
        assert!(format!("{e:#}").contains("permissions"), "{e:#}");
        // With a backup, that stands in instead.
        std::fs::write(sibling(&p, ".bak"), br#"{"n":4}"#).unwrap();
        assert_eq!(load::<V>(&p, "list", &mut w).unwrap(), Some(V { n: 4 }));
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let p = d.join("x.json");
        std::fs::write(&p, b"{}").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&p, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
