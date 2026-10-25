> **Status note (added 2026-10-26).** This audit was run against `4f36ab1`. Its Critical finding —
> destination-folder `exists()` calls under the list lock while adding — is fixed in `c845b50`
> (names chosen before the lock; a media leftover scan moved out; see the DEVLOG entry of the same
> date). The restart, Properties and torrent-removal-wait items are also fixed (`313be73`,
> `43e3f11`, `a955dc4`), and a service log on disk was added (`crates/reidm/src/logfile.rs`).
> The browser-capture paths (the WebSocket add, `/ext/add?verify`) now queue downloads on the
> blocking pool too. Torrent folder placement reads the folder before the lock, and every torrent
> engine call is bounded at two minutes. The remaining findings are follow-ups: a few small
> synchronous reads and writes in media tool setup. Line numbers still refer to `4f36ab1`.

# reIDM 0.8.0 (`feature/parity-and-polish`, HEAD 4f36ab1) — hang / stall audit

Read-only audit of the current 0.8.0 tree for anything that can still block the tokio runtime or
the GUI for a long time, deadlock, wait forever, or grow without bound. Every finding below was
verified by reading the code; each is marked **CONFIRMED** (the mechanism is in the code) or
**UNCERTAIN** (the trigger depends on librqbit/OS/desktop behaviour). The exact user scenarios are
traced in §8.

Files read in full: `reidm-core/src/{lib,media,torrent,store,space,disk,limiter,checksum,power,settings,schedule}.rs`,
`reidm/src/{web,rows,main,ws,session,install}.rs`, `reidm-gui/src/{worker,api,tray,notify,dropped,picker,main}.rs`,
`reidm-gui/src/app/*.rs`. Also read for context: `queue.rs`, `update.rs`, `launch.rs`, `view.rs`,
`model.rs`, `playlist.rs`, `torrent.rs` (GUI).

Severity scale used: **Critical** = can freeze the service (or a download) indefinitely in ordinary
use of the reported scenarios; **High** = multi-second-to-multi-minute unresponsiveness or stall in
those scenarios; **Medium** = slower/leak/robustness issue or needs an unusual setup.

---

## Executive summary

The 0.8 fixes (off-lock saver thread, GUI worker thread, HTTP timeouts, cancel-aware waits, yt-dlp
kill on timeout) are present and work as intended: the saver never serialises under the list lock,
`Pause` wakes network reads and the speed limiter, `spawn_blocking` is used for checksums,
preallocate, rename, cross-drive copies, syncs and — in the HTTP API — removal/purge.

What remains is a different failure shape: the engine's global state is still guarded by
`std::sync::Mutex`, and several **async** code paths take those mutexes synchronously *while other
paths hold them across filesystem calls*. On a slow or hung volume (network share, removable disk,
Windows AV scanning) one slow `stat`/`read_dir` under the list lock freezes every task that needs
the list, including the axum handlers and the download checkpoint tasks, on the tokio worker
threads. That is the one remaining route to "the service is frozen and only a restart helps".

Second group: operations that happen *after* the last byte — the cross-drive copy in `finalize` and
checksum verification — are awaited by the download task with **no cancellation check**, so a
multi-GB download sits at 100 % "Running" for minutes; Pause/Remove do nothing and
Restart/Refresh fail with "still stopping" after 15 s.

Third group: unbounded, cancel-blind waits around torrent teardown/session start, and a GUI command
worker that runs some 10-minute jobs on the same single thread that Pause/Remove use.

---

## 1. Locks, lock order, deadlocks

### 1.0 Inventory (all locks that matter)

| # | Lock | Kind | Taken while holding |
|---|---|---|---|
| 1 | `Inner.db` (`DbCell.db`, `lib.rs:459`, `store.rs:252`) | `std::sync::Mutex<Db>` | — (order root) |
| 2 | `DbCell.changes` (`store.rs:256`) | `std::sync::Mutex<ChangeLog>` | only in `DbGuard::drop`, i.e. under **db** (`store.rs:290`) |
| 3 | `Saver.state` + condvar (`store.rs:443`) | `std::sync::Mutex` | under **db** in `DbGuard::drop` → `saver.request` (`store.rs:302`) |
| 4 | `Inner.live` (`lib.rs:464`) | `std::sync::Mutex<BTreeMap<u64, Live>>` | under **db** everywhere |
| 5 | `Live.plan` (`lib.rs:398`) | `std::sync::Mutex<Plan>` | under **db+live** in `list`/`get_many` (`lib.rs:1017-1031`, `1052-1060`); alone in workers/checkpoint |
| 6 | `Live.removed` (`lib.rs:406`) | `std::sync::Mutex<Option<(Item,bool)>>` | under **db+live** in `remove` (`1532-1537`) and `launch` (`1835-1865`); under **db** in `finalize` (`2215-2218`) |
| 7 | `Live.no_space` (`lib.rs:409`) | `std::sync::Mutex` | under **db+live** in `launch` (`1851`); alone in `stop_for_space` (`426`) |
| 8 | `Inner.settings` (`lib.rs:467`) | `std::sync::Mutex<Settings>` | under **db+live** in `place` (`1288`) — see 1.2; alone elsewhere |
| 9 | `Inner.http` (`lib.rs:470`) | `std::sync::RwLock<Client>` | under **settings** in `update_settings` (`938-943`) |
| 10 | `Inner.when_done` (`lib.rs:472`) | `std::sync::Mutex` | alone; with settings temporarily in `power_look` (`power.rs:161`) |
| 11 | `Inner.refresh` (`lib.rs:476`) | `std::sync::Mutex` | alone |
| 12 | `Inner.warnings` (`lib.rs:463`) | `std::sync::Mutex` | taken before `saver.state` in `warnings()` (`lib.rs:905-906`) |
| 13 | `Limiter.bucket` (`limiter.rs:27`) | `std::sync::Mutex` | under **db+live+settings** in `update_settings`/`set_speed_limit` (`lib.rs:948`, `1582-1583`); never waits on another lock |
| 14 | `media::FAILED` (`media.rs:321`), `media::{YTDLP,DENO,FFMPEG}_LOCK` (`media.rs:315-317`) | std / tokio mutexes | tool fetch only; no manager lock held |
| 15 | `torrent::FORGETTING` (`torrent.rs:138`) | `std::sync::Mutex` | alone (`forget_torrent`, `forgotten`) |
| 16 | `Inner.torrents` (`lib.rs:478`) | `tokio::sync::OnceCell` | async init only |
| 17 | Web `App.{rows, ext, ext_answers}` (`web.rs:74,76,84`) | std mutexes | `rows`: cache → db/live/plan (`web.rs:295-301`); ext/ext_answers alone |
| 18 | GUI `Client.token` (`api.rs:190`) | std mutex | alone; file read inside `renew_token`/`token` |

**Documented and actual order:** `db → live → {plan, removed, no_space}`; `db → changes → saver.state`;
`settings` is taken *before* `db/live` everywhere **except** `place`, and always released before
`db` is taken there. No site holds two of {db, live, plan} in any other order.

### 1.1 CONFIRMED — no lock-order cycle, no guard across an `.await`
Two+ locks are held together only at the sites in the table; the orders above are consistent, so
there is no deadlock cycle. All async methods are `Send` (`tokio::spawn`/axum require it), so a
`std::sync::MutexGuard` cannot be held across an `.await`; the sites that could have done it
(`web::ext_add` `ext_answers` block, `status`' `ext` clone, `ext_ping`) take and drop the guard
inside a block/statement before awaiting. **No finding in this category** apart from 1.2/1.3.

### 1.2 CONFIRMED (near-miss) — `place` takes `settings` while holding `db`+`live`
`Manager::place` runs under both locks (`lib.rs:1304-1309`, `1339-1344`) and reads
`self.0.settings.lock()` (`lib.rs:1288`). `pump` (`lib.rs:1375`), `torrent_limits`
(`torrent.rs:610-613`), `prepare` (`lib.rs:1192`) and `run` (`lib.rs:1988,1993`) all take
`settings` **before** `db`, but each drops the settings guard before acquiring `db`. Today the
order cannot deadlock; it is one added settings read inside `update_settings`' block
(`lib.rs:938-947`, which already holds settings while rebuilding the HTTP client) or inside
`torrent_limits`' settings block away from a real `settings → db` vs `db+live → settings`
deadlock. Because `place` can also block on a slow `unique_path` (below) while holding
`db+live+settings`, the settings lock is effectively behind the filesystem.
Test: a debug-only lock-order assertion (thread-local "held" set) around every `.lock()` of the
five manager mutexes, plus a stress test that runs `add_many` and `update_settings` concurrently.

### 1.3 CONFIRMED — most locks recover from poisoning, the rest panic forever
`DbCell.db` and `DbCell.changes` recover (`store.rs:322-327`, `333`, `370`), as do the web mutexes
and `Client.token`. Everything else uses `.lock().unwrap()` (81 occurrences in `lib.rs` alone):
`live` (`lib.rs:1018,1042,1053,1070,1306,1341,1380,1407,1483,1533,1582,1609,1630`), `plan`
(`lib.rs:1031,1527? no — 1031,2077,2152,2172,2288,2300,2326,2343,2355,2364,2484,2511,2531,2550`),
`settings` (see grep list), `when_done`, `refresh`, `removed`, `no_space`, `torrent::FORGETTING`,
`media::FAILED`, `Saver.state` (`store.rs:463,472,474,358,363`), `Limiter.bucket`.
A panic while any of these is held (an index/underflow/OOM panic, or a panic in a callback) poisons
it, after which every later call panics — including `DbGuard::drop` → `saver.request`, so plain
list changes start to panic. `DbCell.db` survives; `live` does not, which makes `list`, `get`,
`pump`, `pause`, `resume` and `launch` fail permanently. Trigger is **UNCERTAIN** (no reachable
panic was found in the normal paths), but the blast radius is the whole service.
Test: `#[test]` that poisons `live` (catch_unwind around a leaked guard) and asserts the manager
still answers; until then, add a poison-recovering accessor like `DbCell`'s for `live`/`plan`/`settings`.

---

## 2. Blocking work on the async runtime

### 2.1 CRITICAL (CONFIRMED mechanism) — global list locks are held across filesystem calls, and async tasks take them synchronously
`Inner.db`/`live` are `std::sync::Mutex`; blocking `.lock()` inside a tokio task blocks the
**worker thread**, not just the task. The following hold `db` (and mostly `live` too) while calling
the filesystem:

* `Manager::place` → `unique_path` (`lib.rs:1260`, `635-652`): two `exists()` per candidate name,
  in a loop. `add_inner`(`1304-1309`) and `add_many_with` (`1339-1344`) hold `db+live` for the whole
  batch/bulk-add.
* `place` re-adding a finished item: `it.path.exists()` under `db+live` (`lib.rs:1230`).
* `reset_item` (called from `redownload` under `db`, `lib.rs:1430-1433`, and from `place`):
  `part_for` → `beside.exists()` (`1002`), and for media `media::leftovers()` = `std::fs::read_dir`
  (`lib.rs:1461`, `media.rs:776`).
* `run()` after its probe: `unique_path` for the server-provided name under `db`
  (`lib.rs:1996-2010`); `part_for` (`2019`).
* `finalize`'s `rename_target` → `unique_path` under `db` (`lib.rs:2272-2280`).
* `choose_target` for a torrent: `path.exists()` under `db` (`torrent.rs:642`).
* `Manager::list`/`get_many` deep-clone every item under `db`(+`live`+`plan`), including segment
  vectors and torrent file lists (`lib.rs:1016-1024`, `1051-1065`).

On a local SSD these are microseconds. On the setups that produce Windows/macOS "only a restart
recovers": a download folder on a mapped/SMB drive or a sleeping external disk, Windows Defender
scanning each create/stat, or a folder with many name collisions (`unique_path` does O(k)
`exists()` pairs for the k-th item of the same name, i.e. O(k²) stats for a batch). One `stat` on a
dead share can block for tens of seconds or indefinitely; while it blocks, `add_many` holds `db`
and `live`, so:
  * every running download's 3 s checkpoint (`lib.rs:2202`) and media/torrent progress write
    (`media.rs:1289`, `torrent.rs:520`) blocks a runtime worker on `db.lock()`;
  * `web::rows`/`list`/`bulk`/`status` spawn_blocking jobs pile up behind the same lock;
  * with 2 tokio workers (the Windows/macOS laptops in the report), both workers can be stuck in
    `Mutex::lock()`, and the whole service — API included — stops answering. The GUI poller then
    times out (30 s), the window shows a stale list, and only a restart appears to help.

The fix is local, not a refactor: never call `exists`/`read_dir` while holding `db`/`live` — gather
candidate paths first, resolve them outside the lock, then take the lock to record the result; and
make `add_many` do the `unique_path` resolution once, outside the batch lock. For the clone cost,
`get_many` should clone only the fields callers need (rows already do this via `Row::of`).
Test: a `#[cfg(test)]` hook in `unique_path`/`path_for` that sleeps 2 s on the first call, while a
`#[tokio::test(flavor="multi_thread", worker_threads=2)]` asserts `/api/downloads/rows` and a
running download's checkpoint still complete within 100 ms during an `add_many` of 10 items, and
that `Instant`-measured lock hold in `add_many_with` is < 5 ms.

### 2.2 HIGH (CONFIRMED) — browser-capture WebSocket path runs `Manager::add` on the runtime
`ws::serve_conn` calls `session.handle(&m)` inline in the async task (`ws.rs:71`), and
`Session::handle` calls the synchronous `self.mgr.add(r)` for both `T_DOWNLOAD` and `T_INTERCEPT`
(`session.rs:131`, `154`). `add` → `prepare` (settings clone, `create_dir_all`,
`fs::create_dir_all` at `lib.rs:1302`) → `db+live` → `place`/`unique_path` → `apply` (synchronous
`remove_file`/`remove_dir_all`, `lib.rs:1350-1355`) → `pump`. This is the path the *real IDM
extension* and the native host use (port 1001/61001), i.e. the main browser hand-off. A slow
destination folder or a busy list lock stalls a runtime worker; the reply to the extension is
delayed and other downloads' checkpoints on that worker stall.
Test: point the manager at a FIFO/hung mount (or add a test hook to `create_dir_all`) and assert a
`/api/status` request still answers while a WS `T_DOWNLOAD` is in flight.

### 2.3 HIGH (CONFIRMED) — `/ext/add` with `verify` runs `add_checked_inner` on the runtime
`web::ext_add_answer` awaits `app.mgr.add_checked(req, opts)` (`web.rs:958`).
`add_checked_inner` ends in `self.add_inner(...)` (`lib.rs:1148`) for a new file, or
`self.add_with(...)` (`1133`) for a known one, and `add_torrent_file` (`1119`) — all synchronous,
including `create_dir_all`, the `exists()`s under `db+live`, the `apply` deletions and `pump`. The
`verify=true` case is exactly the extension's "the browser is holding its own copy, check the link
first" path, so this is the ordinary browser capture on current builds.
Test: as 2.2, through `/ext/add` with `"verify": true` and a deliberately slow `unique_path`.

### 2.4 HIGH (CONFIRMED) — `restart` deletes the old file/work dir synchronously on the runtime
`web::act_on` awaits `m.restart(id)` directly (`web.rs:449`). `restart` → `redownload` →
`reset_item` under `db` (`lib.rs:1427-1436`) → `apply` (`lib.rs:1350-1365`), which does
`remove_file(part)`, `remove_file(path)` (the finished multi-GB file for a Restart of a Done item),
`remove_file` of every yt-dlp leftover and `remove_dir_all(work_dir)` — on whatever thread called
`restart`, which for `/api/downloads/{id}/restart` and the GUI's Restart is a runtime worker.
`bulk` "restart" loops this per id (`web.rs:570-579`). Pausing/removing during it is not observed.
Test: a unit test that restarts a download whose path is a directory tree with 100k entries, run
inside `#[tokio::test(worker_threads=2)]`, asserting a concurrent `/api/status` answers in <100 ms.

### 2.5 MEDIUM (CONFIRMED) — synchronous filesystem calls in the media/torrent launch paths
All on the launch task (runtime worker), none via `spawn_blocking`:

* `run_media`: `std::fs::create_dir_all(&dir)` and `create_dir_all(&work)` (`media.rs:1016,1029`);
  `leftovers()` = `read_dir` of the download folder on every start (`media.rs:1027`, `776`);
  `write_private` for cookies/proxy config each run (`media.rs:961`, `951-962`); on success
  `remove_dir_all(&work)` and `metadata` (`media.rs:1128-1129`); `ytdlp_from`/`js_runtime`/
  `ffmpeg` do `create_dir_all`, `read_to_string` of the stamp, `write(stamp)`, `remove_file`
  (`media.rs:594-598,608,663,686,731,737`). `unzip` itself is correctly in `spawn_blocking`
  (`media.rs:671,736`).
* `run_torrent`/`add_to_session`: `read_torrent_file` = `std::fs::read` of up to 10 MB
  (`torrent.rs:433`, `126-131`), `keep_torrent_file` = `create_dir_all` + `write` of up to 10 MB
  (`torrent.rs:463`, `171-177`), `create_dir_all(&target.folder)` (`torrent.rs:493`), and
  `set_seeding`'s `read_torrent_file` (`torrent.rs:558`).
* `run()`: `create_dir_all(d)` (`lib.rs:1991`), `part_for`'s `exists()` (`2019`), and
  `disk::mark_from_internet` (`2267`, small).
* `session()`: `create_dir_all(&dir)` (`torrent.rs:205`).

Individually small on a local disk; on the volumes in 2.1 they are seconds and they stall a worker.
Test: same slow-FS hook, asserting launch tasks do not delay `/api/status`.

### 2.6 MEDIUM (UNCERTAIN) — rebuilding the HTTP client while holding `settings` and `http.write`
`update_settings` holds `settings` and takes `http.write()` while calling `http_client(&next)`
(`lib.rs:938-943`); `reqwest::ClientBuilder::build()` initialises TLS and (depending on the
backend/version) may load the system certificate store — on macOS the Keychain, on Windows the cert
store. Any async task calling `settings()`/`prepare`/`run_media` (`settings.lock()`) blocks on a
std mutex inside the runtime for that duration. Usually milliseconds; a locked Keychain can make it
hundreds.
Test: measure `settings()` latency while `update_settings` runs in a loop with a proxy change;
assert <10 ms p99.

### 2.7 What is correctly off the runtime (no finding)
`remove`/`purge` from the HTTP API (`web.rs:455-461` → `blocking`), `add`/`add_with` from
`/api/downloads` and the batch endpoints, `list`, `get_many`, `get`, `settings`, `queue`,
`move`, `refresh-capture`, `free_space` for `/api/status`; checksum (`checksum.rs:117,146`);
preallocate, open, rename, cross-drive copy, `sync_data`, `purge_files` in `launch`, media
tool unzip, power actions, `flush`, `persisted` — all `spawn_blocking`. The saver serialises and
writes on its own OS thread (`store.rs:368-438`).

---

## 3. The saver thread (`store.rs`)

CONFIRMED properties:

* Saving takes `db` only long enough to clone pointers (`store.rs:369-372`); serialisation and
  `write_atomic` + `sync_all` + `sync_dir` happen outside every manager lock (`store.rs:374-376`,
  `499-519`).
* `DbGuard::drop` takes `changes` then `saver.state` while still holding `db` (`store.rs:283-304`);
  the saver never takes `db` while holding `saver.state` (`store.rs:382-406` releases `st` before
  `write_now`, `408`; and `409` re-locks after), so **no cycle**.
* A failed write records the error and retries after 5 s (`store.rs:419-433`); waiters time out
  (`Saver::wait`, `store.rs:472-483`). Callers are bounded: `flush` 10 s (`lib.rs:897-900`),
  `persisted` 10 s in `spawn_blocking` (`lib.rs:978-984`), power's when-done flush 10 s
  (`power.rs:219`).
* A hung `sync_all`/`rename` on a dead volume makes the saver thread sit in the syscall; saves then
  queue (`wanted` grows) but every change still returns immediately (only `request`/`Drop` touch
  `state`, which is free while `write_now` runs), and waiters abandon after 10 s with a warning.
  **Not** a wedge of callers, but the list stops reaching disk for as long as the syscall hangs.
  Save failures are surfaced through `warnings()` (`lib.rs:904-908`) and `/api/status`.
* If the saver thread ever panics, `Saver.state` is poisoned and `request`/`wait`/`close` panic
  from then on; because `request` is called from `DbGuard::drop`, ordinary list changes would start
  panicking (and a second panic during unwinding aborts). Trigger UNCERTAIN; same family as 1.3.
* Cost note (CONFIRMED, Medium): every torrent whose piece progress changes calls `record`
  (`torrent.rs:519-545`) which touches the item every 500 ms; `DbGuard::drop` then schedules a save,
  and the 2 s debounce means the whole `downloads.json` — including every torrent's complete file
  list — is serialised and fsynced about every 2 s for as long as any torrent downloads. The
  `SAVE_SHARE` backoff (`store.rs:393`) keeps it near 1/25 of a core, but it is continuous disk
  write next to the transfer.
Test: seccomp/fault-injection of `fsync` returning after 30 s; assert `pause`, `list`, `remove` and
`/api/status` still answer and that the list is written once after it returns.

---

## 4. Blocking-pool exhaustion

`spawn_blocking` users (grep): `web::blocking` wrapper (`web.rs:130-135`), `space::short_of_space`
(`space.rs:65`) and `retry_waiting_for_space` (`141`), torrent mark-from-internet (`torrent.rs:402`),
media unzip (`media.rs:671,736`) and mark-from-internet (`1138`), checksums (`checksum.rs:117,146`),
power (`power.rs:198,214,219,224,230,244`), `persisted` (`lib.rs:981`), launch purge
(`lib.rs:1869`), part-file open/preallocate (`2030`), checkpoint `sync_data` (`2197`), rename
(`2227`), cross-drive copy (`2244`), every write batch (`2538`), plus `main` shutdown flush
(`main.rs:537`).

The runtime is `#[tokio::main]` default: up to **512** blocking threads. The long holders are:
cross-drive copy (one thread for the whole copy, minutes for tens of GB), checksum (one thread per
verify, 1 h client timeout), purge (`remove_dir_all` of a media work dir), and per-download 3 s
`sync_data`. Normal use cannot reach 512 (max_concurrent ≤ 64, at most 64 checkpoints + 64 copies);
a *local* process holding the API token could issue hundreds of concurrent `/verify` calls, but that
is not the reported scenario.

**No finding here for normal use.** Worth noting only that request handlers share the pool with
these long jobs: if 2.1's slow-volume scenario keeps threads parked in file I/O, the pool itself is
not the limit but every request that uses `blocking()` will queue behind them.
Test (optional): with `max_blocking_threads` lowered to 4 in a test runtime, prove that one slow
copy does not block `/api/downloads/rows` (it will — quantify the queueing before changing anything).

---

## 5. GUI

### 5.1 HIGH (CONFIRMED) — one command thread, and some jobs take minutes
`Worker::start` runs every job on one thread in order (`worker.rs:28-46`); the design note says long
work gets its own thread — but these do **not**:

* `queue_batch` (all bulk adds from Add URL / Import / Batch / grabber) runs
  `client.with_timeout(600 s).batch(body)` on the command thread (`commands.rs:271-278`).
* `export_list_to` (120 s) on the command thread (`commands.rs:334-340`).
* Every `bulk(...)` action — including **pause**, resume, remove, purge, restart, schedule — is a
  `cmd.run` job (`commands.rs:75-84`). The agent it uses is the poller's base client, **10 s**
  (`api.rs:583` → `Client::new(10 s)`; `Worker::start(poller.client.clone(), …)`, `app/mod.rs:174-181`),
  so a slow bulk request blocks the queue for up to 10 s and then reports "Could not …".
* `refresh_props` queues a detail fetch every second on the same thread (`commands.rs:488-507`).

While a 600 s batch is running, a Pause/Remove clicked by the user is queued behind it; the window
still draws (so this is no longer the 0.7 freeze), but the action does not happen for minutes.
This is exactly scenario (b).
Test: `worker.rs` test that enqueues a job blocking on a channel and asserts a second job does not
start until released (documents the ordering), plus move `queue_batch`/`export_list_to` to
`in_background` and test that a `bulk` job runs while a batch is pending.

### 5.2 HIGH (CONFIRMED) — long bulk operations time out client-side while the daemon keeps working
The command client's 10 s timeout applies to `bulk` (`api.rs:289-301`). A bulk purge of hundreds of
playlist videos (many GB, plus a `read_dir` + `remove_dir_all` per item, sequential on the daemon —
`web.rs:570-579`) easily exceeds 10 s. The GUI shows "Could not purge: …" although the daemon
continues deleting; a retry adds more requests, and the list looks unchanged until the next poll.
The same applies to bulk restart when any download needs `stop_and_wait` (up to 15 s per item,
`lib.rs:1515-1525`).
Test: a fake service that delays a bulk response by 15 s; assert the GUI either waits or reports
"still working" rather than an error, and that `remove_ids` removes rows only once the daemon
answers.

### 5.3 HIGH (CONFIRMED for large torrents) — Properties deep-clones the full upload every frame
`props_window` does `let detail = st.detail.clone()` on every frame (`props.rs:22`), where `detail`
is the full `reidm_core::Item`: its `segments: Vec<Segment>` and, for a torrent,
`torrent.files: Vec<TorrentFile>` (each a `String` path). With the Properties window open the app
forces a repaint every 250 ms (`mod.rs:494-497` `waiting()` includes `props.is_some()`), and any
input event adds frames. A torrent of 20–50k files therefore allocates/clones tens of thousands of
strings per frame on the UI thread — tens of milliseconds per frame, before the virtualised list is
drawn. `segment_map` also paints every segment (`widgets.rs:31-42`), which is cheap by comparison.
Test: a synthetic `Detail` with 50k `TorrentFile`s; assert one `props_window` frame (headless egui)
takes <2 ms, or borrow `st.detail` instead of cloning it.

### 5.4 MEDIUM (CONFIRMED) — the visible list is rebuilt and the whole list rescanned on every progress tick
`ViewModel::update`'s key includes `snap.rev` (`view.rs:29,84-86`), and the poller increments
`snap.rev` whenever any row changed (`api.rs:677-684`) — i.e. on every poll (~500 ms) while
anything downloads. Each such update walks all items, rebuilds counts and **sorts all visible
rows** (`view.rs:88-150`); `note_completions` (`lifecycle.rs:176-186`) also walks every item and
clones every `Status` on every new snapshot. With a few hundred rows this is invisible; with tens of
thousands of history rows it is tens of milliseconds of UI-thread work twice a second — the window
stutters during large downloads. (Unchanged frames are cheap; it is the progress-only revision
change that defeats the cache.)
Test: extend the 50k-row test with `rev` incremented between updates and assert the per-update cost
(<5 ms); or key counts/rows on a "list structure + visible fields" revision separate from progress.

### 5.5 MEDIUM (CONFIRMED) — filesystem and clipboard calls on the UI thread
All of these run from `eframe`'s `logic`/`ui` on the main thread:
* `import_path`: `std::fs::read` of the whole file, then parse, via `poll_picker`
  (`commands.rs:305`, `349-373`). A large dropped/imported list freezes the window while reading and
  parsing.
* Export of grabber results: `std::fs::write` of the URL list (`commands.rs:368`).
* `TorrentWindow::use_file`: `std::fs::read` of the picked/dropped `.torrent` (GUI has no 10 MB
  guard; `torrent.rs:178-189`).
* Dropped shortcut files: `metadata` + `read_to_string` (bounded to 64 KB, `intake.rs:166-170`).
* Clipboard reads on menu/dialog open (`mod.rs:162-164`, `menu.rs:18`, `commands.rs:573`) and on
  `Copy` (`intake.rs:68-71`); on X11/Wayland these are round trips and can block if the clipboard
  owner is unresponsive.
No daemon call is made on the UI thread (poller, command worker, `in_background` threads cover
those); that part of the 0.8 fix is intact.
Test: a UI-thread watchdog thread that records the longest gap between `logic` runs over a 5 s
window while importing a 50 MB list; assert <50 ms.

### 5.6 UNCERTAIN — tray updates can be a D-Bus round trip per rate change
`Tray::show_state` only runs when the state changes (`tray.rs:174-177`), but the state includes the
formatted transfer rate (`lifecycle.rs:94-104`), which changes on every 500 ms snapshot while a
download runs. Each change calls `set_tooltip` and several `CheckMenuItem` setters
(`tray.rs:178-193`). On Linux with `tray-icon`/StatusNotifierItem these may go over D-Bus; whether
that happens on the UI thread or a helper thread depends on the crate backend. If synchronous, it
is a 2 Hz UI-thread round trip to the desktop shell (a hung shell would freeze the window).
Test: on a Linux desktop with a StatusNotifier host, measure `show_state` with a changing tooltip;
if >2 ms, quantise the tooltip rate.

### 5.7 Poller and notifications (no finding)
The poller is its own thread with a 30 s client (`api.rs:583-590`); `ensure_daemon` (up to 15 s) runs
there (`api.rs:114-150,604`); notifications run on spawned threads (`lifecycle.rs:207-213`);
yt-dlp/playlist/inspect/verify/grab/refresh all use `in_background` threads with their own timeout
(`api.rs:341-413`); the file picker is on its own thread (`picker.rs:35-54`); the clipboard watcher
is a separate thread (`intake.rs:20-57`). `Worker` replies are applied on the UI thread but are
small.

---

## 6. Unbounded growth per download/delete

### 6.1 MEDIUM (CONFIRMED) — kept `.torrent` files are never deleted
`keep_torrent_file` writes `data/torrents/files/<hash>.torrent` (`torrent.rs:171-177`) for every
torrent added from a file/bytes and for every magnet once its metadata arrives (`torrent.rs:463`).
`Manager::remove`/`purge_files` only calls `forget_torrent` (`lib.rs:1562-1565`,
`torrent.rs:584-603`), which deletes the torrent from the librqbit session; no code removes the
stored `.torrent` file (grep confirms). Add/remove cycles therefore grow the data folder by up to
10 MB each (`MAX_TORRENT_FILE`, `torrent.rs:97`) and never shrink it.
Test: add a `.torrent`, remove it with files, assert `data/torrents/files` no longer holds it.

### 6.2 LOW/MEDIUM (CONFIRMED) — completed segment vectors are kept and cloned forever
`Segment`s are only appended (`claim`, `lib.rs:678-705`) and never merged or dropped on completion;
they stay in the item and are serialised into `downloads.json`. `list`/`get`/`get_many` deep-clone
them under the list lock (`lib.rs:1016-1024`, `1051-1065`), and `web::list` serialises them. Typical
downloads end with a handful; a very fast server racing several connections can split down to
`MIN_PIECE` (256 KiB) and, on a resumed/very large file, leave hundreds of segments per item. Not a
hang, but it multiplies `downloads.json` size and the cost of every full list clone. (Splitting is
self-limiting: it always halves the largest remaining range, so the count grows like
`connections × log(size/min)`, not `size/min`.)
Test: a fast local server + 16 connections on a 4 GB file; assert the final segment count is
< 1000 and that `downloads.json` for it is < 100 KB.

### 6.3 LOW (CONFIRMED) — GUI clipboard bookkeeping is session-unbounded
`ClipboardSeen.offered` grows with every distinct link copied while watching (`dropped.rs:101-130`),
and `App.clip_offers` (`mod.rs:143`) grows if the user copies links without answering the prompt
(one is shown at a time). Memory only, small per entry.
Test: offer 100k links; assert memory/len stays bounded if `offered` is capped.

### 6.4 What is correctly bounded (no finding)
`ChangeLog` at 65,536 ids (`store.rs:86,290-300`); `RowCache.removed` at 4,096 (`rows.rs:25,178-189`);
`ext_answers` at 4,096 with 10-minute expiry (`web.rs:38-43,889-901`); media work dirs removed on
success/reset/remove (`media.rs:1128`, `lib.rs:1460-1462,1556-1561`); part files removed on
purge/reset; torrent `FORGETTING` pruned (`torrent.rs:591`); GUI `Watch`, `Rates`, `completed`,
`selected` pruned against the list (`notify.rs:53`, `api.rs:558-559`, `lifecycle.rs:183-184`);
proxy/cookie temp files removed by `Drop`/after the run (`media.rs:918-924`, `1117-1119`); the
download list itself copy-on-write with a single saver thread (`store.rs`).

---

## 7. Timeout coverage

### 7.1 HIGH (CONFIRMED) — `forgotten()` waits forever, and Pause cannot break it
`run_torrent` starts with `self.forgotten().await` (`torrent.rs:328`); `forgotten` waits on every
pending removal receiver with `rx.wait_for(|done| *done).await` (`torrent.rs:315-320`) — no timeout,
no `live.cancel` check. The pending removal is a spawn from `forget_torrent`, which runs
`s.delete(id, delete_files).await` (`torrent.rs:594-602`); librqbit's delete can take a long time
for a large torrent (it removes the files), and if it ever hangs, `run_torrent` never returns. The
live entry then stays forever: `stop_and_wait` fails after 15 s (`lib.rs:1515-1525`), `pause` sets a
flag nobody reads, `remove` marks the item removed and cancels but `launch` never reaches the purge,
and the concurrency slot is lost until the process restarts. This is the "restart is the only fix"
shape on the torrent side. Reachable from "Restart from the beginning" on a torrent: `reset_item`
forgets it with `delete_files=true` (`lib.rs:1455-1457`, `1357-1360`) and immediately `pump`s the
reset download, which then waits on its own deletion.
Test: `forgotten` with a receiver whose sender is kept open and never sends; assert it gives up (or
observes cancel) within `STOP_LIMIT`, and that `remove`/`pause` on a torrent in that state still
finish.

### 7.2 HIGH (CONFIRMED) — end-of-download `finalize` and `verify_checksum` ignore cancellation
After the last byte, `run` awaits `finalize` then `verify_checksum` (`lib.rs:2160-2161`).
`finalize`'s cross-drive branch copies the whole file (`std::fs::copy` + `sync_all`,
`lib.rs:2244-2247`) inside one blocking job with no progress, no timeout and no `cancel` check;
`verify_checksum` hashes the whole file (`checksum.rs:114-134`, `117`) the same way. The live entry
and the concurrency slot stay held, status stays `Running` at 100 %, Pause does nothing, Remove's
purge waits for the run to end, and Restart/Refresh (`stop_and_wait`, 15 s) fail with "still
stopping; try again in a moment". On Windows the default temp folder (`%APPDATA%\reidm\parts`) and
a download folder on `D:`/another drive make the cross-drive copy the normal path, and a 10–50 GB
copy takes minutes; with a checksum it is followed by a full read. This matches "large downloads
stall" closely.
Test: a cross-device test (temp on one tmpfs, destination on another) that sets `cancel` during the
copy and asserts the copy is interrupted within, say, 1 s and the status becomes Paused; likewise a
cancellable hash loop (check an `AtomicBool` every MiB).

### 7.3 MEDIUM (CONFIRMED / partly UNCERTAIN) — other long or unbounded waits
* `watch_ytdlp` after stdout ends loops on `child.wait()` with a 250 ms timer that only checks
  `cancel` (`media.rs:1231-1241`): if yt-dlp (or a child it waits on) never exits, the download
  stays `Running` forever. Pause works (kill_tree, ≤10 s); no deadline otherwise. **CONFIRMED**.
* `set_seeding` calls `s.add_torrent(...).await` with no timeout and no `unless_stopped`
  (`torrent.rs:563`); for a magnet that no peer describes, the HTTP handler (and each id in a bulk
  "seed") can wait indefinitely. `inspect_torrent` wraps the same call in a 120 s timeout
  (`torrent.rs:296-298`) precisely because of this. **CONFIRMED**.
* `session()`'s `Session::new_with_opts` (librqbit session creation, UPnP port forwarding enabled,
  `torrent.rs:229-235`) is awaited with no timeout and no cancel in `run_torrent` (`327`) and in
  handlers (`inspect_torrent`, `set_seeding`). Whether it can block for minutes is **UNCERTAIN**
  (depends on librqbit/UPnP/DHT), but there is no upper bound in our code.
* `run_torrent`'s cancel path awaits `s.pause(&handle).await` with no timeout (`torrent.rs:366`);
  if librqbit's pause stalls, the run and its slot stall. **UNCERTAIN**.
* Disk writes/flushes (`write_out`'s spawn_blocking, `checkpoint`'s `sync_data`) are not
  interruptible; a hung volume keeps the worker and the run forever. Inherent to blocking I/O, but
  note it is exactly what a stuck network mount does. **CONFIRMED mechanism, environmental trigger**.
* `stop_and_wait` is the only bounded wait for another task to stop: 15 s, polled every 50 ms
  (`lib.rs:1515-1525`). `launch`'s cleanup `spawn_blocking(purge).await` (`lib.rs:1869`) and
  `run()`'s `JoinSet` (workers with per-request timeouts) are the other waits on tasks.

### 7.4 Covered waits (no finding)
Engine HTTP: connect timeout per request (`lib.rs:2391-2397`), reqwest read timeout and an explicit
body idle timer reset per chunk (`lib.rs:2458-2475`), `If-Range`/`Content-Range` checks, retries with
cancel-aware naps (`lib.rs:731-737`). Probe: 12 s in `add_checked` (`lib.rs:1135-1138`) plus client
connect/read timeouts; `run`'s probe is cancel-aware (`lib.rs:1978-1981`). `.torrent` fetch: 30 s
request + 30 s stall (`torrent.rs:181-184`, `102-123`); inspect: 120 s. yt-dlp: 300 s for `-J`
(`media.rs:178-184`), 20 s/30 s + 3 tries for tool downloads (`media.rs:276-277,364-378,396-429`),
`--abort-on-unavailable-fragments`, `kill_on_drop`, process group, taskkill with a 5 s bound
(`media.rs:847-871`). `unless_stopped` polls cancel every 100 ms (`media.rs:758-763`,
`torrent.rs:742-747`). Saver waits 10 s. `flush` 10 s. GUI/CLI clients: 10 s/30 s/120 s/600 s/3600 s
as above; `ensure_daemon` 15 s; notification/tray/picker on their own threads.

---

## 8. The two user scenarios traced

### (a) Multi-GB HTTP download with the GUI polling
Steady state is sound: the GUI poller asks `GET /api/downloads/rows?since=…` every 500 ms; the
daemon answers from `RowCache`, cloning only changed/live items inside `spawn_blocking`
(`web.rs:286-309`); the transfer's 8 connections pace on the limiter, write through
`spawn_blocking`, and checkpoint every 3 s with `sync_data` in `spawn_blocking` (`lib.rs:2190-2208`);
Pause wakes reads, the limiter and retries within milliseconds.

Where seconds can appear:
1. **Completion, cross-drive** (7.2): after the last byte the file is copied and synced with no
   progress and no cancellation; status stays `Running` at 100 % for the whole copy. Pause/Remove
   do nothing; Restart/Refresh fail after 15 s. On Windows this is the default when downloads go to
   a second drive.
2. **Completion, checksum given** (7.2): the whole file is read with the same properties.
3. **A concurrent bulk operation** (2.1/8b): if anything holds `db+live` across a slow filesystem
   call, every checkpoint and every GUI poll blocks on the lock; with two runtime workers the API
   can stop answering, the poller's 30 s timeout expires, and the window shows a stale list.
4. **Slow `sync_data`** on NTFS with a large dirty page cache can take seconds; it is off-runtime,
   but the checkpoint ticker and the final `recorded?` wait on it, so progress in the window
   appears to pause and the slot is held a little longer.
5. GUI-side, with a large history: the per-snapshot full rebuild/sort and completion scan
   (5.4) cause stutter twice a second.

### (b) Bulk-add a few hundred items and bulk-remove them while one large download runs
Add:
* GUI: `queue_batch` runs on the single command thread with a 600 s timeout (5.1), so any
  Pause/Remove clicked meanwhile waits behind it.
* Daemon: one `/api/downloads/batch` → `add_many_with` in `spawn_blocking`
  (`web.rs:1083-1091`) holding `db+live` for the whole batch while `place`→`unique_path` does two
  `exists()` per item (more on name collisions, up to O(k²) stats) (`lib.rs:1260,635-652`). On a
  local SSD this is milliseconds; on a network/AV-scanned folder it is seconds, and during it every
  checkpoint/progress write and every HTTP request needing the list blocks — the 2.1 freeze.
* The running download's workers do not need `db`; they keep transferring, but their checkpoint
  (3 s) and the torrent/media progress loops stall, so the window's `downloaded` stops moving.
* `prepare`/`create_dir_all` happen before the lock, but `place` also takes `settings` under the
  lock (1.2).

Remove:
* GUI: one `/api/downloads/bulk` on the command thread with the **10 s** client (5.2). `get_many`
  clones every named item (segments + torrent file lists) under `db+live` (`web.rs:547`,
  `lib.rs:1051-1065`) — heavy for 10k ids with large torrents.
* Daemon: for each id, `remove` under `db+live`, then `save_now()` + `pump()` per id
  (`lib.rs:1530-1549`) — one **urgent full-list serialise+fsync per removed item** (hundreds of
  full `downloads.json` writes, competing with the download's writes), a `pump` scan of all items,
  and, for non-live items, `purge_files` in `spawn_blocking` (sequential per id): part file,
  `leftovers` `read_dir` of the download folder, `remove_dir_all(work_dir)`, the file itself
  (`lib.rs:1552-1572`). Deleting hundreds of playlist videos/partial work dirs takes far more than
  10 s; the GUI reports failure while the daemon continues.
* Live items are only marked `removed` + cancelled; their purge is done by `launch`'s cleanup in
  `spawn_blocking` (`lib.rs:1867-1869`), but that cleanup is awaited before the task's
  `save_now()`/`pump()`, so starting the next queued download is delayed by the deletion.
* If the large download finishes during this, its own `finalize`/checksum (7.2) holds its slot and
  its cleanup's purge queues behind the bulk deletions in the blocking pool.

Net: the service does not deadlock in this scenario, but the bulk requests run for minutes, the GUI
reports a failure at 10 s, hundreds of redundant full-list fsyncs hit the same disk as the running
download, and any slow `stat` under the list lock can escalate to a service-wide freeze.

---

## 9. Suggested tests (summary)

1. Debounced test hooks in `unique_path`/`path_for`/`read_dir` wrappers; assert list-lock hold time
   < 5 ms and API latency < 100 ms during bulk adds (2.1, 2.2, 2.3, 2.4, 2.5).
2. Cross-device copy test with cancellation: pause/remove during `finalize` must interrupt within
   ~1 s (7.2).
3. Checksum loop that checks `cancel` every MiB; abort a 4 GB hash on remove (7.2).
4. Torrent `forgotten` with a never-completing delete receiver: bounded wait and cancel-aware
   (7.1); restart-a-torrent test that reaches `forgotten` and then removes it.
5. GUI worker test with a 600 s-style blocking job and a pause job behind it; then move bulk
   add/export to background threads (5.1).
6. GUI fake daemon delaying a bulk response 15 s; assert no premature "Could not …" (5.2).
7. Headless egui frame test for Properties with 50k torrent files (5.3) and 50k list rows with
   `rev` changing (5.4).
8. Torrent add/remove test asserting `data/torrents/files` is emptied (6.1).
9. Lock-order assertion instrumentation around the five manager mutexes under a concurrent
   add/update_settings/remove/pump stress test (1.2).

---

## 10. Categories with no finding

* **1 (deadlock/lock-order cycle):** none found. No cycle; no `MutexGuard` across an `.await`; the
  only concern is 1.2 (near miss) and 1.3 (poisoning) above.
* **4 (blocking-pool exhaustion):** not reachable in normal use (512 default threads; at most ~128
  long jobs at the documented concurrency limits; only a valid-token local script could flood
  `/verify`).
* **7 (timeout coverage) for the engine HTTP path:** complete (connect/read/idle timeouts, cancel
  selects, bounded retries, bounded probes and tool fetches).
* **6 (growth) for the change log, row cache, request-id cache, removed/notifying/GUI maps:** all
  capped or pruned; only the `.torrent` store (6.1) and retained segment vectors (6.2) were found.
