# reIDM download engine audit (`crates/reidm-core`)

Static, read-only review of `feature/torrents` @ a5ad036. Nothing was built or run. Line numbers refer
to that commit. Each finding was checked against the code; anything not fully verified is marked
**(uncertain)**. Library behaviour that was checked in the vendored sources: tokio 1.53.2
`fs::File::poll_write` (it returns `Ok` before the write happens; the error shows up on the next
call) and reqwest 0.12.28 (`ClientBuilder::read_timeout` exists).

Severity scale: **Critical**: silent data corruption or data loss in normal use. **High**: corruption
or loss in plausible failure cases, a hang, or a serious scaling problem. **Medium**: wrong behaviour
in edge cases, or noticeable performance or memory cost. **Low**: polish.

---

## Critical

### C1. Progress is counted before the bytes are written, and the "is the data on disk?" check can never fail
- **Where:** `lib.rs:1892-1899` (`write_all`, then `s.done += take`), `lib.rs:1614-1633` (resume check, then `set_len`), `lib.rs:1707-1713` (checkpoint).
- **What goes wrong:** `tokio::fs::File::write_all` returns as soon as the chunk has been copied into tokio's buffer and a blocking write has been queued. The write's result only appears on the *next* write or flush (tokio `file.rs:751-801`). So `segment.done` is ahead of the disk by one chunk per connection, and the 3-second checkpoint saves that number.
  - **Disk full / EIO (Linux, macOS):** `set_len` makes a sparse file, so ENOSPC can arrive in the middle of a download. The failed chunk has already been added to `done`. The next write returns the error, the io error is fatal, and the final checkpoint saves `done` including bytes that never reached the disk. The user frees space and resumes.
  - **Crash / kill / power loss:** the same in-flight chunk, plus anything still in the page cache. The documented safety check (`on_disk < max(start+done)`, ARCHITECTURE.md "Persistence") **can never fire after the first run**, because the first run calls `set_len(size)` (`lib.rs:1632`), so the part file is always full length. The only case it catches is a deleted or truncated part file (which is all `resume_after_the_part_file_is_lost` tests).
  - **Result:** the resumed download finishes "successfully" with a block of zeros inside it. There is no end-to-end check (no hash, no size-only check would catch it).
- **Fix:**
  1. Write through `std::fs::File` + `FileExt::write_at` / `seek_write` inside `spawn_blocking` (or one blocking writer per connection fed by a channel). Advance `done` only after the write has returned `Ok`.
  2. Keep a separate `durable` offset per segment. The checkpoint calls `sync_data()` on the part file (outside the db lock), then copies `durable = done` into the saved list. Only `durable` is persisted.
  3. Drop the length-based check, or keep it only for the deleted-file case. Document that the guarantee now comes from the fsync before each checkpoint.
- **Test:** a writer that fails the Nth write with `ENOSPC` (inject a write fn behind a trait, or use a tiny tmpfs/loop device on CI). Then resume, then compare the result byte for byte with the source. Also: kill the process (child-process test) right after a checkpoint, resume, and compare bytes.

---

## High

### H1. `downloads.json` writes are not crash-safe, and loading fails into "empty list" or "won't start"
- **Where:** `lib.rs:834-842` (`save`), `lib.rs:679-682` (load), `lib.rs:824-832` and `698-701` (settings).
- **What goes wrong:**
  - The temporary file is never fsynced before the rename (and the directory is never fsynced). After a power cut on XFS, btrfs or NTFS, `downloads.json` can be zero-length or garbage. `serde_json::from_slice` then fails with `"corrupt downloads.json"`, `Manager::open` returns an error, and **the daemon refuses to start**. The browser hand-off then dies too, until the user deletes the file and loses the whole list.
  - `Err(_) => Db { next_id: 1, .. }` treats *every* read error (EACCES, a sharing violation from antivirus or backup on Windows, EIO) as "no list". The first `save()` afterwards renames an empty list over the real one. Example: the file became root-owned after a `sudo reidm ...` run, and the next normal start wipes the user's list.
  - A corrupt `settings.json` silently falls back to defaults (`.ok()`). That resets `temp_dir`, so every download whose part file lives in the custom temporary folder restarts from zero (see M9) and leaks its old part file.
  - `save()` ignores all errors without logging (disk full means the list silently stops being saved).
- **Fix:** write the temporary file, `sync_all()`, rename, fsync the parent directory (Unix). Keep the previous file as `downloads.json.bak`. On load, treat only `NotFound` as empty. On a parse error, fall back to `.bak`, and if that fails, rename the bad file to `downloads.json.corrupt-<ts>` and start empty *with a loud warning*. Never fail startup. Do the same for settings. Log save failures.
- **Test:** unit tests for `open_with` with (a) a 0-byte `downloads.json` and a valid `.bak`, (b) an unreadable file (chmod 000 on Unix): must error or use `.bak`, never overwrite, (c) truncated JSON.

### H2. File I/O and whole-list serialisation run while the `Mutex<Db>` is held, on async worker threads, several times a second
- **Where:** `lib.rs:834-842` (guard held through `to_vec_pretty`, `fs::write` and `rename`). Callers: the per-download checkpoint every 3 s (`1674-1682`), `launch` (`1464`, `1488`), `set_note` (`1515`), media progress every 3 s (`media.rs:796`), torrents every 5 s (`torrent.rs:329`), `watch_seeding` every 3 s for as long as anything seeds (`torrent.rs:381`), and **every** `add_*`.
- **What goes wrong:** each save pretty-prints the *entire* list (cookies of several KB per item, every segment of every item) while holding the db lock, then writes it synchronously on a tokio worker thread. Rough estimate: with 5,000 items at about 3 KB each, that is about 15 MB per save, roughly 30-80 ms of CPU plus the write. With 4 downloads that means about 1.5 saves/s even when idle-ish; seeding with ratio < 0 means one rewrite every 3 s forever.
  - On a 2-core machine tokio has 2 worker threads. When one is inside `save()` and the other blocks on `db.lock()` (the GUI calls `list()` 3×/s, plus `pump`, `record`, `set_note`), **the whole runtime stops**: no socket reads, no API. On an HDD, `write()` can stall for hundreds of ms under dirty-page throttling, so the GUI stutters and throughput drops.
  - It also wears out SSDs: about 10-20 MB/s of JSON rewrites from a large list.
- **Fix:** one saver task. Callers set a `dirty` flag and `Notify`. The saver runs at most once per ~2 s (and immediately on status changes, if wanted): it takes the lock, serialises with **compact** `to_vec` (or snapshots `Arc<Item>`s), releases the lock, then writes and fsyncs in `spawn_blocking`. Longer term: persist live segment progress separately (a small per-download file, or SQLite/redb), so a checkpoint does not rewrite thousands of finished items. Drop segments (and optionally cookies) of `Done` items.
- **Test:** a benchmark-style test with 5,000 synthetic items and 4 fake running downloads, asserting `list()` latency stays under N ms (or count saves per second through a test hook).

### H3. No timeout on getting the response headers: a download can hang forever, Pause does nothing, and the slot stays taken
- **Where:** `lib.rs:720-728` (client: only `connect_timeout`), `lib.rs:1811` (`rb.send().await` in `transfer`), `lib.rs:1534` (`probe` from `run` and from `refresh_address`; only `add_checked` wraps it in `PROBE_LIMIT`).
- **What goes wrong:** `IDLE_LIMIT` and the cancel checks only start once the body is streaming. If a server accepts the connection and never answers, or if (very commonly) a worker sends a new range request over a pooled keep-alive connection that died in a Wi-Fi switch or NAT timeout, then `send()` waits until the kernel gives up on TCP retransmits (~15 min on Linux) or forever. During that time:
  - `pause()` has no effect (cancel is not checked);
  - `stop_and_wait` gives up after 15 s (`lib.rs:1212-1217`), and `restart` / `refresh_address` then change the item while the old run is still live. When the old run finally ends, its checkpoint writes the old segments back and its status (`Paused`) overwrites the `Queued` set by the restart (`1474-1486`);
  - the download keeps one of the `max_concurrent` slots.
- **Fix:** `ClientBuilder::read_timeout(60s)` (per read, so it does not limit total length), `.tcp_keepalive(30s)`, `.pool_idle_timeout(30s)`. Wrap `send()` and `probe()` in `tokio::select!` against a cancellation future: replace the `AtomicBool` + polling with `tokio_util::sync::CancellationToken` or a `Notify`. Give `refresh_address`'s probe the same `PROBE_LIMIT`.
- **Test:** test server mode "accept, read the request, never reply". Assert that pause gives `Paused` within ~1 s, and that a running download fails over to retry within the read timeout.

### H4. The retry counter never resets after progress, so long downloads on flaky links lose connections and then fail
- **Where:** `lib.rs:1749-1797` (`attempt` only ever increases), `lib.rs:1734-1743` (a worker that runs out of attempts leaves the pool for good).
- **What goes wrong:** every transient error, even one after 500 MB of good data, uses up one of the segment's 5 retries. Over a multi-hour download on a flaky line each connection drops out after its 6th hiccup, handing its range back and **never coming back**. Speed falls toward one connection. When the last worker hits its 6th error, the download is `Failed`. There is no retry at the manager level, so the user has to resume it by hand. The same thing happens with servers that cap the length of a ranged response (some CDNs): every response ends "early" (`1915-1917`), so each one costs an attempt.
  - The 429/503 path (`1785-1787`) also removes connections permanently, so a short overload spike leaves the rest of the run on one connection.
- **Fix:** reset `attempt = 0` whenever the transfer advanced `done`. Treat a clean early end of a 206 whose `Content-Range` end was smaller than asked as normal (continue from the new offset). Add a supervisor in `run()` that tops the worker count back up to `connections` every ~30 s while unclaimed or splittable work remains. Optionally add download-level auto-retry with backoff for non-fatal failures (IDM does this).
- **Test:** a server that cuts every response after 256 KiB (clean close with a correct Content-Length, or a capped `Content-Range`). A 20 MiB download must finish. Also a server that resets 10 connections spread over the transfer.

### H5. Finished files silently replace whatever is already at the destination
- **Where:** `lib.rs:567-577` (`move_into_place`: `rename` replaces on Unix *and* on Windows, where std uses `MOVEFILE_REPLACE_EXISTING`), `lib.rs:1584-1588` vs `1600-1605` (the path picked in `run` after the probe is chosen under one lock and saved under another), `lib.rs:546-563`.
- **What goes wrong:**
  1. `unique_path` is checked when the download is added or probed. Days later, when it finishes, the file is renamed over whatever was created at that path since: the browser's own copy, a file the user saved, or another download.
  2. Two downloads whose probes finish at about the same time with the same server name (`Content-Disposition: report.pdf`) both build `claimed` before either one has saved its new path. Both pick `report.pdf`, and the second to finish destroys the first. The part files differ (id prefix), so nothing warns about it.
- **Fix:** pick the path and save it under a single db lock. In `move_into_place`, use a rename that refuses to replace (`renameat2(RENAME_NOREPLACE)` / `linkat`+`unlink` on Linux, `MoveFileExW` without `REPLACE_EXISTING` on Windows). If the destination exists, pick a new name with `unique_path` and update `item.path`. Only `redownload` should replace on purpose.
- **Test:** add a download, create the destination file yourself before it completes, and assert both files survive. Two downloads with the same `Content-Disposition` name finishing together must end with two files.

### H6. Batch adds are O(N²): parse every URL, clone every path and save the whole list for each link
- **Where:** `lib.rs:1005` and `940` (`same_download` calls `url::Url::parse` for *every* item on every add), `1047` (clones every path into a `HashSet` on each add), `1079` (full save on each add), `1089-1096` (`add_many_with` loops over `add_with`), `1406` (same scan in `refresh_address`).
- **What goes wrong:** a 10,000-link pattern (`MAX_PATTERN_ITEMS`) or an `.ef2` import into a 5,000-item list means about 75M URL parses, 75M `PathBuf` clones and 10,000 whole-list serialisations (each up to tens of MB), all under the db lock. On a low-end PC that is minutes of a frozen daemon, and it lags even on a fast one.
- **Fix:** store the `url_key` in `Item` (computed once, `#[serde(default)]` and rebuilt on load). Keep `HashMap<String, Vec<u64>>` key→ids and `HashSet<PathBuf>` claimed paths in `Db` (not serialised). Have `add_many_with` take the lock once and save once (or use the debounced saver from H2).
- **Test:** add 10,000 URLs to a manager that already has 10,000 items. Assert it takes under 2 s (and exactly one save, through a counter hook).

### H7. NTFS pre-allocation causes huge synchronous zero-fills (Windows, mostly HDD) **(semantics well known; not measured here)**
- **Where:** `lib.rs:1631-1633` (`set_len(size)`) together with the segments starting at `k*size/n`.
- **What goes wrong:** on NTFS, `SetEndOfFile` moves EOF but not *ValidDataLength*. The first write at offset X forces Windows to zero-fill from VDL up to X before the write returns. With 8 segments on a 4 GB file, the last segment's first write zero-fills about 3.5 GB: around 35 s on an HDD, during which that connection does not read (the server may drop it, which uses up a retry; see H4) and the disk is saturated for every other download.
- **Fix:** on Windows, mark the part file sparse (`FSCTL_SET_SPARSE`) before `set_len`. Or skip `set_len` and let the file grow, writing segments in increasing order. `SetFileValidData` is an option only when the process has `SE_MANAGE_VOLUME_NAME`. On Linux use `fallocate` (it reserves space, which also avoids most of C1's ENOSPC-in-the-middle case and reduces HDD fragmentation).
- **Test:** a Windows CI job that downloads a 2 GB local file with 8 connections and asserts no single write stalls for more than 2 s (time the writer).

---

## Medium

### M1. "Pause" does nothing on a download that is queued but not started
- **Where:** `lib.rs:1193-1197`; `web.rs:195` and `session.rs:174` call it directly.
- **What goes wrong:** `pause()` only sets `cancel` for downloads in `live`. A `Queued` row waiting for a slot stays `Queued` and starts by itself later. The user pressed Pause and the download runs anyway.
- **Fix:** in `pause()`, under the db lock and then the live lock (the order the code requires), set `Queued` → `Paused` when the item is not live (the logic already exists in `stop_and_wait`, `1203-1209`).
- **Test:** `max_concurrent=1`, add 2 downloads, pause #2, finish #1, and assert #2 is `Paused` and never ran.

### M2. Resume pressed while a download is stopping is lost
- **Where:** `lib.rs:1142-1144` (`resume` returns `Ok` when the item is live, even if it is cancelling), `lib.rs:1015-1029` (same in `add_inner`), `lib.rs:1484` (the end of the run sets `Paused` unconditionally).
- **What goes wrong:** Pause then Resume within the shutdown window (up to seconds, unbounded with H3) ends `Paused`.
- **Fix:** in `resume`, if the item is live and `cancel` is set, mark it `Queued` (or set a `requeue` flag on `Live`). At the end of `launch`, if the status is `Queued` (or the flag is set), keep it queued and let `pump` restart it.
- **Test:** start a throttled download, call `pause`, then `resume` straight away, and assert it ends `Running`/`Done`.

### M3. `AlreadyComplete` is reported even when the file has been deleted, so the browser gives the download up
- **Where:** `lib.rs:1016-1018` (the docstring at `100-101` promises "the file is still on disk"; the code never checks).
- **What goes wrong:** the user deletes `setup.exe` and clicks the link again. The extension hands it over, reIDM answers `already_complete`, and the extension cancels the browser's download (`service-worker.js:453` shows "Already downloaded"). The file cannot be downloaded again from the browser.
- **Fix:** `Done && !it.path.exists()` (for torrents, check the folder) → `redownload` and return `Existing`/`New`.
- **Test:** download, delete the file, re-add the same URL, and assert a new transfer happens.

### M4. Re-adding a link does not refresh its cookies, referer or user agent
- **Where:** `lib.rs:1005-1029` (only `filename` is copied, and only when it was unset).
- **What goes wrong:** the usual reason a download failed is an expired session. Clicking the link again (as the docs suggest) resumes it with the *old* cookie and it fails again.
- **Fix:** when an existing, not-live item is re-added with `cookie`/`referer`/`user_agent`, replace them (the same address, so it is safe).
- **Test:** a server that needs `Cookie: s=2`. Add with `s=1` (fails), re-add with `s=2`, and assert `Done`.

### M5. The global speed limit is not global
- **Where:** `media.rs:706-714` (each yt-dlp gets the full global rate, frozen when it starts), `torrent.rs:141` and `442` (librqbit has its own bucket set to the same rate).
- **What goes wrong:** with a 1 MB/s limit, 4 videos + HTTP downloads + torrents can pull more than 6 MB/s. Changing the limit does not reach running yt-dlp processes.
- **Fix:** give each yt-dlp `global / running_media` and restart it (it resumes) on large changes, or proxy its traffic. For torrents, split the global budget, for example by taking the torrent session's measured rate off the HTTP bucket. At least document it.
- **Test:** a limit of 200 KB/s, one HTTP and one fake-yt-dlp download, assert the combined rate.

### M6. Tool fetching holds one global lock with no timeout and ignores Pause; failed update checks are retried on every video
- **Where:** `media.rs:266`, `404`, `445`, `477` (`TOOLS` taken *before* the fast "already installed" check), `303-310` (engine client: no timeout), `648` (first cancel check comes after every tool step), `408-441` (the timestamp is written only on success).
- **What goes wrong:** while ffmpeg (~100 MB) downloads over a slow link, or the request stalls (see H3), every other video download waits on `TOOLS` even though yt-dlp is installed, and Pause on any of them does nothing. Offline or behind a captive portal, each video start waits for `latest_tag` (3 tries, up to 20 s each, plus 8 s of pauses) one after another under the lock: 20 queued videos means many minutes.
- **Fix:** check the fast path before taking the lock. Use one lock per tool. Give fetches a request timeout and a `read_timeout`. Race each tool step against cancellation. Write a "last attempt" timestamp on failure as well (back off for an hour).
- **Test:** a fake GitHub that hangs. Assert a second video with yt-dlp already present starts at once, and that pausing a video stuck fetching tools returns `Paused` within 1 s.

### M7. Whole tool archives and unbounded `.torrent`/page bodies are read into memory
- **Where:** `media.rs:306` (`r.bytes().await?.to_vec()` copies the whole body a second time), `media.rs:342-381` (each entry is inflated to its own Vec, and stored entries get `raw.to_vec()`), `lib.rs:925-929` (the 16 MiB cap applies only when `Content-Length` is present: a chunked response is read in full for 30 s), `grabber.rs:408-412` (same for pages).
- **What goes wrong:** the ffmpeg zip costs about 2× its size at peak (~200+ MB) on a 4 GB machine. A hostile `.torrent` link that sends an endless chunked body can push GBs into RAM within the 30 s timeout at gigabit speed.
- **Fix:** stream to a file in `tools/` while hashing with `Sha256::update`, then unzip from the file. Read `.torrent` and grabber bodies with `bytes_stream()` and stop at the cap.
- **Test:** a server that sends a chunked `.torrent` of 64 MiB. Assert an error and memory below the cap (count the bytes read).

### M8. Hot path: one blocking-pool hop, two mutex locks and a timer per network chunk, and small writes
- **Where:** `lib.rs:1848-1907`.
- **What goes wrong:** reqwest chunks are about 8-16 KiB (TLS records). Each one costs a `timeout()` timer, 2× `plan` locks, the limiters (no lock when unlimited, which is good) and a tokio `File` write, which means a `spawn_blocking` hop with only one write in flight per file handle. At 1 GB/s that is about 65-130k hops per second (roughly a core of overhead), and the per-connection rate is limited by the round-trip time of that hop. On an HDD, 32 streams of 16 KiB writes interleaved across 4 files (default 8 connections × 4 downloads) cause seek storms during writeback.
- **Fix:** collect 256 KiB-1 MiB per connection before claiming and writing (positional `write_at` in `spawn_blocking`, which also fixes C1). Keep one `Sleep` for the idle timer and reset it instead of creating a timeout per chunk. Update `plan` once per batch.
- **Test:** a throughput benchmark against a local server (criterion or an ignored test): 4 GiB with 16 connections, recording CPU per GB before and after.

### M9. Part files are found from the *current* temporary-folder setting
- **Where:** `lib.rs:847-862` (`part_for` is recomputed every time), `1226` (`remove`).
- **What goes wrong:** change the temporary folder (or lose `settings.json`, see H1) and every unfinished download starts again from 0 (`on_disk=0`). Its old multi-GB part file is never deleted, not even by Remove.
- **Fix:** save `part: PathBuf` in `Item` when the part file is created, and use that from then on. On a temporary-folder change, offer to move unfinished parts.
- **Test:** start a download, pause, change `temp_dir`, resume, and assert it continues from the same offset (or at least that the old part is gone after Remove).

### M10. Torrents: name collisions get overwritten, a restart races with the delete, duplicate info hashes slip through
- **Where:** `torrent.rs:243-251` (`overwrite: true` for *new* torrents, into the category folder, with no `unique_path`), `lib.rs:1165-1168` + `torrent.rs:423-435` (`forget_torrent` is spawned and not awaited, while `redownload` immediately calls `pump`), `lib.rs:130-133` (`url_key` keeps the btih as written: hex and base32 forms of the same torrent get different keys).
- **What goes wrong:**
  - A new torrent named `debian.iso` overwrites (after hash-checking) a different `debian.iso` already in the folder, whether from an HTTP download or another torrent with the same name.
  - Restart: the new `run_torrent` adds the torrent while the old delete is still pending. librqbit answers "already managed" with the old handle, and then the spawned `delete(…, true)` removes it **and its files** under the new run. **(uncertain about exact librqbit behaviour; the race window is real)**
  - Adding a base32 magnet of a torrent already added from a `.torrent` file makes a second row that shares one session torrent, so removing either with its files deletes the other's data.
- **Fix:** use `overwrite: true` only when `item.torrent` is already `Some` (resume). Otherwise pick a free name, or a subfolder. Make `forget_torrent` async and await it in `restart`/`redownload`. Normalise btih to lowercase hex in `url_key` (base32 → hex).
- **Test:** an existing unrelated file with the torrent's name must survive. Restart a finished torrent and assert it downloads again. A hex and a base32 magnet must be one row.

### M11. Many open-ended `Content-Range`/length checks are missing
- **Where:** `lib.rs:1822-1834`, `1835-1837`.
- **What goes wrong:**
  - A `206` without `Content-Range` is accepted, so the start offset is never checked.
  - For a non-ranged (`resumable=false`) download of known size, a response with a different `Content-Length` is cut at the old size, or its tail is silently ignored, and the result is reported `Done`, a mix of the new file and the old size.
  - `416` on resume (the file shrank) goes through 5 retries and fails with a raw HTTP error instead of `ChangedOnServer`.
  - Only a strong ETag is used for `If-Range`. A server with only `Last-Modified` gets no change detection when the size is the same.
- **Fix:** require `Content-Range` on 206 and check that both start and end are within `[from, end]`. For 200, compare `content_length()` with `ctx.size` and raise `ChangedOnServer` on a mismatch. Map 416 to `ChangedOnServer`. Fall back to `If-Range: <Last-Modified>` when there is no ETag.
- **Test:** test-server modes for: a 206 with no `Content-Range`; a 200 with a different length on retry; a 416 on resume.

### M12. `downloads.json` holds session cookies with default permissions (0644)
- **Where:** `lib.rs:838` (and `media.rs:719-725`: the cookie file is written first and only *then* chmodded, and is left behind if the process crashes).
- **What goes wrong:** on shared Linux or macOS machines with a 0755 home directory, other users can read every saved cookie. The API token next to it is carefully 0600, so this is inconsistent.
- **Fix:** create the temporary file with `OpenOptions::mode(0o600)` (Unix). Create the yt-dlp cookie file with mode 0600 at once, and in the work directory, so it is cleaned up with it. Clear cookies when an item becomes `Done` (or after N days).
- **Test:** on Unix, assert the mode of `downloads.json` after `add` is 0600.

### M13. Memory growth per item and per poll
- **Where:** `lib.rs:871-889` (`list()` deep-clones every item, including cookies and all segments, 3×/s for the GUI; `get()` does a whole `list()` just to find one item and is used by `run`, `refresh_address` and `web.rs:202`), `torrent.rs:359-364` (`watch_seeding` clones the whole list every 3 s, for ever once the session exists), `torrent.rs:448-450` (`peers_of` turns the full `TorrentStats`, including per-file progress, into a `serde_json::Value` every 500 ms per torrent), and finished items keep their cookies and segments for ever.
- **Fix:** `get()` should look up the item directly. Store items as `Arc<Item>` (cheap snapshot clones), or add a version counter so the API can return 304. When a download completes, empty its segments vector and drop the cookie. Read the peer count from the typed stats fields instead of a JSON round trip.
- **Test:** count allocations (dhat) for 1,000 `list()` calls on a 5,000-item list, before and after.

---

## Low

- **L1. Paths too long or names too long.** `sanitize_filename` (`lib.rs:463-477`) never limits length. A `Content-Disposition` name of 245-255 bytes works as the final name but fails as the part file `"{id}-{name}.reidm"` (the 255-byte component limit on ext4 and NTFS), so the download fails with "File name too long". *Fix:* truncate to about 200 bytes on a char boundary, keeping the extension. Name part files `{id}.reidm`. *Test:* a 300-character name.
- **L2. `url_key` can confuse different URLs.** It joins *decoded* query pairs without re-encoding: `?a=1%26b%3D2` and `?a=1&b=2` collide, and sorting merges `?id=2&id=1` with `?id=1&id=2`. *Fix:* use `form_urlencoded::Serializer` and a stable sort on the key only. *Test:* add these to `url_identity_ignores_fragment_and_query_order`.
- **L3. Grabber `include` filter requires *all* patterns** (`grabber.rs:285`), while the docstring says "one of". *Fix:* `!include.is_empty() && !include.iter().any(|p| s.contains(p))`. Also, `extract_links` allocates 6 `Vec<char>` at *every* character position (`grabber.rs:158-160`), and `blank_non_markup` searches the rest of the page again on every loop (O(n·k)). *Fix:* precompute the patterns and remember the next match positions.
- **L4. Refresh does not update `resumable` or segments** (`lib.rs:1439-1451`). Refreshing a download that has been probed but has no bytes yet onto a non-range server keeps `resumable=true`, which leads to "server stopped honouring Range" ×5. *Fix:* set `it.resumable = p.resumable` and rebuild the segments when nothing has been downloaded.
- **L5. A removed download can recreate its part file.** If the user removes it between the probe's save (`1600-1606`) and the open (`1614`), the part file is created and pre-allocated to full size and never deleted. *Fix:* check that the item still exists (under the lock) after creating the part file, and delete it if not.
- **L6. Pausing leaves limiter debt.** `Limiter::take` charges the tokens before waiting (`limiter.rs:52`). Pausing a heavily throttled download leaves up to `connections×chunk` of debt in the *global* bucket, which stalls every other download for seconds at low limits. *Fix:* refund what was not used on an early return. Downloads with more connections also get a proportionally larger share (no fairness between downloads). That is acceptable but worth documenting.
- **L7. On startup, queued downloads become `Paused`** (`lib.rs:688-689`), so a queue of 96 waiting downloads does not carry on after a reboot. This is probably intentional; consider keeping `Queued` (or an option to resume on start).
- **L8. No graceful shutdown.** `ctrl_c` simply returns (`main.rs:444-447`) without stopping transfers or doing a final checkpoint and fsync. SIGTERM (logout, systemd) is not handled. With C1 fixed this costs only progress, not correctness. Still, add a shutdown hook that cancels, waits up to 2 s, and saves.
- **L9. yt-dlp output edge cases** (`media.rs:771-823`). A read error on stdout (invalid UTF-8) `break`s out of the loop and then `child.wait()` waits with stdout unread: yt-dlp blocks on a full pipe and the download hangs with Pause disabled. *Fix:* read lines as bytes and decode lossily; race `child.wait()` against cancellation. On Windows, `taskkill` runs synchronously on a runtime thread (`media.rs:604-607`): use `tokio::process`. `kill_on_drop` kills only the direct child, not the group.
- **L10. A partial ffmpeg unzip is treated as installed** (`media.rs:446-448`). If the unzip stops midway (disk full), `ffmpeg.exe` exists without its DLLs and every merge fails from then on. *Fix:* extract into `ffmpeg.tmp/`, then rename the directory.
- **L11. A failed copy across drives leaves a partial destination** (`lib.rs:574`), and the copy of a multi-GB file shows no progress. *Fix:* copy to `dest.reidm-move`, then rename; delete it on error.
- **L12. Right-to-left override characters are allowed in names.** `sanitize_filename` removes only `Cc` controls. U+202E (RTLO) makes `invoice‮fdp.exe` display as `invoiceexe.pdf`. *Fix:* also strip bidi controls (U+202A-U+202E, U+2066-U+2069).
- **L13. `update::check` uses `reqwest::blocking` inside `#[tokio::main]`** (`main.rs:745`): reqwest documents that this panics when the client is dropped in an async context. **(uncertain: verify with `reidm update-check`)**. *Fix:* make `check` async, or call it from `spawn_blocking`.
- **L14. Scheduler and DST.** In the hour that repeats when the clocks go back, `now <= prev` returns `None` and `prev` moves backwards, so a start or stop in that hour fires twice. That is harmless today. Forward jumps and missing hours are handled correctly. FAT32 destinations: `set_len` > 4 GiB fails with a raw `EFBIG`; pre-check and say so.

---

## Code quality (the parts that matter)

- **`lib.rs` is 2,000 lines, with `run()` about 145 lines and `transfer()` about 120** mixing the protocol, file I/O and plan bookkeeping. Split it into `store.rs` (Db + saver), `http.rs` (probe/transfer), `plan.rs` (claim/split, already pure and well tested) and `writer.rs` (positional writes + durability). C1, H2 and M8 all become local changes after that split.
- **Duplicated logic:** the file-name hint is worked out twice in `add_inner` (`984-990` vs `1035-1046`, with different fallbacks for media). Adding a torrent is duplicated between `run_torrent` (`238-258`) and `set_seeding` (`395-404`). The "build `claimed` + `unique_path`" sequence appears in two places with different locking.
- **Cancellation by polling** an `AtomicBool` (250 ms naps in `nap`, `Limiter::take`, `run_torrent`, `run_media`) instead of a `CancellationToken`. This is the root cause of H3, M6 and L9.
- **A `std::sync::Mutex` held across blocking I/O** in an async program (H2). Rule: no fs calls and no serialisation of the whole list under `db`.
- **Missing tests for important behaviour:** crash consistency and disk full (C1); load or corruption recovery (H1); a server that never answers (H3); retries over a long transfer and servers that cap ranges (H4); destination collisions at move time (H5); scaling of batch adds (H6); Pause on queued items (M1); re-add after deleting the file, and with new cookies (M3/M4); torrent name collisions and restart (M10). The existing tests (dynamic split, 429 servers, If-Range change, refresh, zero-byte files, duplicate names at add time) are good and cover the happy-path design well.

## Things checked and found correct

- Dynamic split arithmetic: `claimed` protects the in-flight write; after a split the halves cover the range exactly with no overlap (`claim`, `lib.rs:603-630`). The test proves it.
- `pump` reserves slots under db+live, so two concurrent pumps cannot start the same download twice. Lock order (db → live → plan, db → settings) is followed everywhere I looked; I found no inversion.
- A download removed during the probe is not brought back (`lib.rs:1600-1605`).
- Reading a non-resumable download again resets `done` and progress correctly.
- No transparent decompression; HTTP/1.1 only (no `http2` feature), so segmenting really opens separate TCP connections.
- Scheduler window logic, including overnight windows, chosen days and the case of sleeping through both start and stop.
