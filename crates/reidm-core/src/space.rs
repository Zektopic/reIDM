//! Not filling the disk: a download stops, keeping everything it has, before it would leave a
//! drive with less free space than the user asked to keep, and starts again by itself once
//! there is room.

use crate::{disk, human, media, Item, Manager, Status, SPACE_RETRY_EVERY};
use std::collections::HashMap;
use std::path::PathBuf;

impl Manager {
    /// What `it` still has to write, and where: `(folder, bytes)` per drive it writes to. A
    /// size not known yet counts as one byte — it will need *some*.
    fn space_needs(&self, it: &Item) -> Vec<(PathBuf, u64)> {
        let Some(dest) = it.path.parent().map(PathBuf::from) else { return Vec::new() };
        let left = it.size.map_or(1, |s| s.saturating_sub(it.downloaded));
        if it.request.torrent.is_some() {
            // The torrent engine writes straight into the download's folder.
            return vec![(dest, left)];
        }
        // Files and videos are put together in the temporary folder, then moved.
        let (temp, here) = if it.request.media.is_some() {
            (media::work_dir(&self.temp_dir(), it.id), left)
        } else {
            let part = self.part_for(it);
            let here = match it.size {
                // Space already reserved for the part file is not needed again.
                Some(size) if part.exists() => disk::unreserved(&part, size, it.downloaded),
                _ => left,
            };
            (part.parent().map(PathBuf::from).unwrap_or_else(|| self.temp_dir()), here)
        };
        if disk::volume(&temp) == disk::volume(&dest) {
            // Moved within the drive: the move itself takes no room.
            vec![(temp, here)]
        } else {
            // Copied to another drive at the end: the whole file is needed there too.
            vec![(temp, here), (dest, it.size.unwrap_or(1))]
        }
    }

    /// Why `it` can't carry on without taking a drive below the space to keep free, if it
    /// can't. Asks the file system: call it off the async runtime.
    pub(crate) fn space_shortage(&self, it: &Item) -> Option<String> {
        let keep = self.0.settings.lock().unwrap().min_free_space_mib.saturating_mul(1 << 20);
        self.space_needs(it).into_iter().filter(|(_, need)| *need > 0).find_map(|(dir, need)| {
            let free = self.free_space_at(&dir)?;
            (free < need.saturating_add(keep)).then(|| {
                // How much *more* free space the drive needs for the download to fit above
                // the floor — not how many bytes the download still has to write, which with
                // the size not known yet would read as "needs 1 B more".
                let short = need.saturating_add(keep).saturating_sub(free);
                format!(
                    "not enough space on {}: needs {} more free space, {} free (reIDM keeps {} free; it carries on by itself when there is room)",
                    dir.display(),
                    human(short),
                    human(free),
                    human(keep)
                )
            })
        })
    }

    /// [`Manager::space_shortage`], without holding up the runtime.
    pub(crate) async fn short_of_space(&self, it: &Item) -> Option<String> {
        let (me, it) = (self.clone(), it.clone());
        tokio::task::spawn_blocking(move || me.space_shortage(&it)).await.ok().flatten()
    }

    fn free_space_at(&self, dir: &std::path::Path) -> Option<u64> {
        #[cfg(test)]
        if let Some(f) = self.0.faults.free_space() {
            return Some(f);
        }
        disk::free_space(dir).ok()
    }

    /// The free space where downloads go and where unfinished ones are kept, for the status bar.
    pub fn free_space(&self) -> Vec<(PathBuf, Option<u64>)> {
        let dirs = [self.settings().default_dir, self.temp_dir()];
        dirs.into_iter().map(|d| (d.clone(), disk::free_space(&d).ok())).collect()
    }

    /// Start the downloads stopped for want of space that now fit, first in the queue first.
    /// Each one started counts against its drive's free space, so ten waiting downloads don't
    /// all start on room for one. Returns how many were started.
    pub fn retry_waiting_for_space(&self) -> usize {
        let waiting: Vec<Item> = {
            let db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let mut v: Vec<Item> = db.items.values().filter(|i| i.waiting_for_space && i.status == Status::Paused && !live.contains_key(&i.id)).map(|i| (**i).clone()).collect();
            v.sort_unstable_by_key(|i| (i.order, i.id));
            v
        };
        if waiting.is_empty() {
            return 0;
        }
        let keep = self.0.settings.lock().unwrap().min_free_space_mib.saturating_mul(1 << 20);
        // Per drive: what is free, less what the downloads started here will use.
        let mut free: HashMap<Option<String>, Option<u64>> = HashMap::new();
        let mut start = Vec::new();
        for it in &waiting {
            let needs = self.space_needs(it);
            let fits = needs.iter().all(|(dir, need)| {
                let f = *free.entry(disk::volume(dir)).or_insert_with(|| self.free_space_at(dir));
                f.is_some_and(|f| f >= need.saturating_add(keep))
            });
            if fits {
                for (dir, need) in &needs {
                    if let Some(Some(f)) = free.get_mut(&disk::volume(dir)) {
                        *f = f.saturating_sub(*need);
                    }
                }
                start.push(it.id);
            }
        }
        {
            let mut db = self.0.db.lock().unwrap();
            for id in &start {
                // Unless the user did something with it meanwhile.
                if let Some(it) = db.items.get_mut(id).filter(|i| i.waiting_for_space && i.status == Status::Paused) {
                    it.status = Status::Queued;
                    it.note = None;
                }
            }
        }
        if !start.is_empty() {
            tracing::info!("{} download(s) waiting for space carry on", start.len());
            self.save_now();
            self.pump();
        }
        start.len()
    }

    /// Look for room for the downloads waiting for space every [`SPACE_RETRY_EVERY`].
    pub(crate) fn spawn_space_watch(&self) {
        let me = std::sync::Arc::downgrade(&self.0);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SPACE_RETRY_EVERY).await;
                let Some(inner) = me.upgrade() else { return };
                let m = Manager(inner);
                let _ = tokio::task::spawn_blocking(move || m.retry_waiting_for_space()).await;
            }
        });
    }
}
