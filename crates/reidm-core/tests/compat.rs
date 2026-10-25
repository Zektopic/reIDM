//! A list saved by an earlier reIDM must still load, and its unfinished downloads must carry on
//! from where they were — not start again, and never splice in wrong bytes.

use reidm_core::{Manager, SettingsPatch, Status};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 253) as u8 ^ (i >> 12) as u8).collect()
}

/// Serves one file with ranges, and records every `Range` asked for.
fn serve(data: Vec<u8>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let seen = ranges.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let (data, seen) = (data.clone(), seen.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut range = None;
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 {
                            return;
                        }
                        if h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            seen.lock().unwrap().push(v.trim().to_string());
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap_or(data.len() - 1).min(data.len() - 1)));
                        }
                    }
                    let (a, b) = range.unwrap_or((0, data.len() - 1));
                    let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\n\r\n", data.len(), b + 1 - a);
                    if s.write_all(head.as_bytes()).is_err() || s.write_all(&data[a..=b]).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, ranges)
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reidm-compat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `downloads.json` exactly as reIDM 0.7.0 wrote it (pretty-printed, every field it had and
/// none it didn't): one finished download, and one paused half way with two ranges.
fn list_070(port: u16, dl: &Path, size: usize, done: [usize; 2]) -> String {
    let half = size / 2;
    let dl = dl.to_string_lossy().replace('\\', "\\\\");
    format!(
        r#"{{
  "next_id": 3,
  "items": {{
    "1": {{
      "id": 1,
      "request": {{ "url": "http://127.0.0.1:{port}/old.bin", "referer": null, "cookie": null, "user_agent": null, "filename": null, "dir": "{dl}" }},
      "path": "{dl}/old.bin",
      "size": 10,
      "downloaded": 10,
      "status": {{ "state": "Done" }},
      "segments": [ {{ "start": 0, "end": 9, "done": 10, "finished": false, "active": false }} ],
      "resumable": true,
      "added": 1760000000,
      "last_try": 1760000000,
      "finished_at": 1760000001,
      "etag": null,
      "last_modified": null,
      "speed_limit": null,
      "scheduled": false
    }},
    "2": {{
      "id": 2,
      "request": {{ "url": "http://127.0.0.1:{port}/big.bin", "referer": "http://127.0.0.1:{port}/", "cookie": "sid=1", "user_agent": null, "filename": null, "dir": "{dl}" }},
      "path": "{dl}/big.bin",
      "size": {size},
      "downloaded": {downloaded},
      "status": {{ "state": "Running" }},
      "segments": [
        {{ "start": 0, "end": {e0}, "done": {d0}, "finished": false, "active": true }},
        {{ "start": {half}, "end": {e1}, "done": {d1}, "finished": false, "active": true }}
      ],
      "resumable": true,
      "added": 1760000000,
      "last_try": 1760000100,
      "finished_at": null,
      "etag": null,
      "last_modified": null,
      "speed_limit": null,
      "scheduled": false
    }}
  }}
}}"#,
        downloaded = done[0] + done[1],
        e0 = half - 1,
        e1 = size - 1,
        d0 = done[0],
        d1 = done[1],
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_from_0_7_0_loads_and_its_downloads_carry_on() {
    let size = 2_000_000;
    let data = body(size);
    let (port, ranges) = serve(data.clone());
    let dir = tmp("070");
    let (state, dl) = (dir.join("state"), dir.join("dl"));
    std::fs::create_dir_all(state.join("parts")).unwrap();
    std::fs::create_dir_all(&dl).unwrap();
    let done = [300_000, 250_000];
    std::fs::write(state.join("downloads.json"), list_070(port, &dl, size, done)).unwrap();
    std::fs::write(state.join("settings.json"), format!(r#"{{ "default_dir": "{}", "connections": 2, "max_concurrent": 4 }}"#, dl.to_string_lossy().replace('\\', "\\\\"))).unwrap();
    // 0.7.0's part file: `<id>-<name>.reidm` in the temporary folder, full length, holding the
    // bytes its ranges count and zeros elsewhere.
    let mut part = vec![0u8; size];
    let half = size / 2;
    part[..done[0]].copy_from_slice(&data[..done[0]]);
    part[half..half + done[1]].copy_from_slice(&data[half..half + done[1]]);
    std::fs::write(state.join("parts/2-big.bin.reidm"), &part).unwrap();
    std::fs::write(dl.join("old.bin"), b"0123456789").unwrap();

    let m = Manager::open(&state, dl.clone(), 8).unwrap();
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(m.settings().connections, 2, "saved settings win");
    let items = m.list();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].status, Status::Done);
    assert_eq!(items[1].status, Status::Paused, "what was running when reIDM stopped is paused");
    assert_eq!(items[1].downloaded, 550_000);

    m.resume(2).unwrap();
    wait_until("done", || m.get(2).unwrap().status == Status::Done);
    assert!(std::fs::read(dl.join("big.bin")).unwrap() == data, "the resumed file differs");
    // It carried on from the two ranges' progress rather than starting again.
    let asked = ranges.lock().unwrap().clone();
    assert!(asked.iter().any(|r| r.starts_with(&format!("{}-", done[0]))), "{asked:?}");
    assert!(!asked.iter().any(|r| r.starts_with("0-") && r != "0-0"), "started again from byte 0: {asked:?}");
    // The new id continues after the old ones, and the list it saves still reads as before.
    assert!(m.flush());
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(state.join("downloads.json")).unwrap()).unwrap();
    assert_eq!(saved["next_id"], 3);
    assert_eq!(saved["items"]["1"]["status"]["state"], "Done");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_list_is_recovered_from_its_backup_and_reported() {
    let dir = tmp("damaged");
    let state = dir.join("state");
    {
        let m = Manager::open(&state, dir.join("dl"), 4).unwrap();
        m.add_with(reidm_core::Request { url: "http://127.0.0.1:9/a.bin".into(), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() }).unwrap();
        assert!(m.flush());
        m.add_with(reidm_core::Request { url: "http://127.0.0.1:9/b.bin".into(), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() }).unwrap();
        assert!(m.flush());
    }
    // A power cut left the list empty.
    std::fs::write(state.join("downloads.json"), b"").unwrap();
    let m = Manager::open(&state, dir.join("dl"), 4).unwrap();
    // The backup is the save before the last one: it has the first download, and has the
    // second too when the saver wrote it twice.
    let urls: Vec<String> = m.list().into_iter().map(|i| i.request.url).collect();
    assert!(urls.iter().any(|u| u.ends_with("/a.bin")), "{urls:?}");
    assert_eq!(m.warnings().len(), 1);
}

/// The temporary folder on another drive than the download folder: the finished file is
/// copied across, and nothing at the destination is replaced.
#[tokio::test(flavor = "multi_thread")]
async fn a_finished_file_moves_across_drives() {
    let shm = Path::new("/dev/shm");
    let other = std::env::temp_dir();
    let dev = |p: &Path| -> Option<u64> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(p).ok().map(|m| m.dev())
        }
        #[cfg(not(unix))]
        {
            let _ = p;
            None
        }
    };
    let (Some(a), Some(b)) = (dev(shm), dev(&other)) else { return };
    if a == b {
        return; // no second file system to move across here
    }
    let data = body(1_500_000);
    let (port, _) = serve(data.clone());
    let dir = tmp("xdev");
    let dl = dir.join("dl");
    let parts = shm.join(format!("reidm-xdev-{}", std::process::id()));
    let m = Manager::open(&dir.join("state"), dl.clone(), 2).unwrap();
    // No free-space floor: this is about moving across file systems, and /dev/shm is often far
    // smaller than the default floor (64 MB in a container), where the download would rightly
    // wait for space instead.
    m.update_settings(SettingsPatch { temp_dir: Some(parts.clone()), min_free_space_mib: Some(0), ..Default::default() }).unwrap();
    let id = m.add(reidm_core::Request { url: format!("http://127.0.0.1:{port}/x.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    // Taken while it downloads: it must survive.
    std::fs::create_dir_all(&dl).unwrap();
    std::fs::write(dl.join("x.bin"), b"keep me").unwrap();
    wait_until("done", || matches!(m.get(id).unwrap().status, Status::Done | Status::Failed(_) | Status::Paused));
    let it = m.get(id).unwrap();
    assert_eq!(it.status, Status::Done, "{:?}", it.note);
    assert_eq!(std::fs::read(dl.join("x.bin")).unwrap(), b"keep me");
    assert!(std::fs::read(m.get(id).unwrap().path).unwrap() == data);
    let left: Vec<_> = std::fs::read_dir(&dl).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left.len(), 2, "no temporary copy left behind: {left:?}");
    assert_eq!(std::fs::read_dir(&parts).unwrap().count(), 0, "the part file goes once the move is done");
    let _ = std::fs::remove_dir_all(&parts);
}

/// Files exactly as 0.8's first phase wrote them (every field it had, none it didn't): they load
/// unchanged, the new settings take their defaults, the queue gets an order in the order the
/// downloads were added, and the files written back still read as before.
#[tokio::test(flavor = "multi_thread")]
async fn files_from_0_8_phase_1_load_with_the_new_defaults() {
    let dir = tmp("p1");
    let (state, dl) = (dir.join("state"), dir.join("dl"));
    std::fs::create_dir_all(&state).unwrap();
    let d = dl.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        state.join("settings.json"),
        format!(
            r#"{{
  "default_dir": "{d}",
  "connections": 8,
  "max_concurrent": 4,
  "speed_limit": 0,
  "limit_enabled": false,
  "schedule": {{ "enabled": false, "start": null, "stop": null, "days": [] }},
  "sort_by_category": true,
  "category_dirs": {{}},
  "temp_dir": null,
  "seed_ratio": 1.0,
  "upload_limit": 0,
  "torrent_port": 6881,
  "torrent_dht": true
}}"#
        ),
    )
    .unwrap();
    let item = |id: u64, state: &str| {
        format!(
            r#""{id}": {{ "id": {id}, "request": {{ "url": "http://127.0.0.1:9/{id}.bin", "referer": null, "cookie": null, "user_agent": null, "filename": null, "dir": "{d}" }},
            "path": "{d}/{id}.bin", "size": null, "downloaded": 0, "status": {{ "state": "{state}" }}, "segments": [], "resumable": false,
            "added": 1760000000, "last_try": null, "finished_at": null, "etag": null, "last_modified": null, "speed_limit": null, "scheduled": true,
            "part": "{d}/{id}.bin.reidm" }}"#
        )
    };
    // Ids out of order on purpose: the order follows the id, not the file's order.
    std::fs::write(state.join("downloads.json"), format!(r#"{{ "next_id": 8, "items": {{ {}, {}, {} }} }}"#, item(7, "Paused"), item(2, "Queued"), item(5, "Paused"))).unwrap();

    let m = Manager::open(&state, dl.clone(), 8).unwrap();
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    let s = m.settings();
    assert_eq!(s.performance, reidm_core::Performance::Balanced);
    assert_eq!((s.write_buffer_kib, s.retries, s.retry_wait_secs, s.idle_timeout_secs, s.connect_timeout_secs, s.min_free_space_mib), (256, 5, 3, 60, 30, 200));
    assert!(s.keep_awake && !s.keep_awake_seeding);
    assert_eq!(s.proxy, reidm_core::Proxy::default());
    assert_eq!(m.queue_order(), [2, 5, 7], "in the order they were added");
    let items = m.list();
    assert!(items.iter().all(|i| i.checksum_ok.is_none() && !i.waiting_for_space && i.request.checksum.is_none()));
    // A new download goes to the back of the queue.
    let new = m.add_with(reidm_core::Request { url: "http://127.0.0.1:9/new.bin".into(), dir: Some(dl), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() }).unwrap().id;
    assert_eq!(m.queue_order().last(), Some(&new));

    // What is written back is the same shape, with the order kept.
    assert!(m.flush());
    m.update_settings(SettingsPatch { keep_awake: Some(false), ..Default::default() }).unwrap();
    drop(m);
    let m = Manager::open(&state, dir.join("elsewhere"), 8).unwrap();
    assert_eq!(m.queue_order(), [2, 5, 7, new]);
    assert!(!m.settings().keep_awake);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(state.join("downloads.json")).unwrap()).unwrap();
    assert_eq!(saved["next_id"], 9);
    assert!(saved["items"]["2"]["order"].as_u64().unwrap() < saved["items"]["5"]["order"].as_u64().unwrap());
}
