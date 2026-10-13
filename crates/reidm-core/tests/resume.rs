//! End-to-end checks against a real (in-process) HTTP server.
//!
//! These pin the two ways a *resume* used to silently produce a corrupt file that was then
//! reported as complete: a server that ignores `Range`, and a part file that no longer holds the
//! bytes the database says it does.

use reidm_core::{Manager, Request, Status};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Deterministic body so a corrupted result is obvious.
fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

struct Server {
    port: u16,
    _handle: std::thread::JoinHandle<()>,
}

/// `ranges`: honour `Range` requests. `throttle`: bytes per second (0 = unthrottled).
fn serve(data: Vec<u8>, ranges: bool, throttle: usize) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { break };
            let data = data.clone();
            std::thread::spawn(move || handle_conn(s, &data, ranges, throttle));
        }
    });
    Server { port, _handle: handle }
}

fn handle_conn(mut s: TcpStream, data: &[u8], ranges: bool, throttle: usize) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line.trim().is_empty() {
            break;
        }
        let mut start = 0usize;
        let mut end = data.len().saturating_sub(1);
        let mut want_range = false;
        if ranges && line.to_ascii_lowercase().starts_with("range:") {
            if let Some(spec) = line.split_once("bytes=").map(|(_, v)| v.trim().to_string()) {
                let (a, b) = spec.split_once('-').unwrap_or(("", ""));
                start = a.trim().parse().unwrap_or(0);
                end = if b.trim().is_empty() { data.len().saturating_sub(1) } else { b.trim().parse().unwrap_or(end) };
                end = end.min(data.len().saturating_sub(1));
                want_range = true;
            }
        }
        // Consume the rest of the header block.
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            if h.trim().is_empty() {
                break;
            }
        }
        let slice = if start <= end && start < data.len() { &data[start..=end] } else { &data[0..0] };
        let head = if want_range {
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                data.len(),
                slice.len()
            )
        } else {
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n", slice.len())
        };
        if s.write_all(head.as_bytes()).is_err() {
            return;
        }
        if throttle == 0 {
            if s.write_all(slice).is_err() {
                return;
            }
        } else {
            let chunk = (throttle / 20).max(4096);
            for c in slice.chunks(chunk) {
                if s.write_all(c).is_err() {
                    return;
                }
                if s.flush().is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = s.flush();
    }
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reidm-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Wait until `f` holds; panics with `what` if it never does.
fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn state(m: &Manager, id: u64) -> Status {
    m.get(id).expect("item").status
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_on_a_server_that_ignores_range_is_not_corrupt() {
    let data = body(3_000_000);
    let srv = serve(data.clone(), false, 600_000); // ~0.6 MB/s, no Range support
    let dir = tmp("norange");
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();

    let id = m
        .add(Request { url: format!("http://127.0.0.1:{}/file.bin", srv.port), dir: Some(dir.join("dl")), ..Default::default() })
        .unwrap();

    // Let it get partway, then stop it.
    wait_until("progress", || m.get(id).unwrap().downloaded > 100_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);
    assert!(m.get(id).unwrap().downloaded > 0, "should have made progress before the pause");

    m.resume(id).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);

    let got = std::fs::read(m.get(id).unwrap().path).unwrap();
    assert_eq!(got.len(), data.len(), "length must match");
    assert!(got == data, "resumed file must be byte-identical to the source");
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_after_the_part_file_is_lost_is_not_corrupt() {
    let data = body(3_000_000);
    let srv = serve(data.clone(), true, 600_000);
    let dir = tmp("lostpart");
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();

    let id = m
        .add(Request { url: format!("http://127.0.0.1:{}/file.bin", srv.port), dir: Some(dir.join("dl")), ..Default::default() })
        .unwrap();

    wait_until("progress", || m.get(id).unwrap().downloaded > 100_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);

    // Simulate the part file being deleted or lost with a power cut: the database still claims
    // progress, but the bytes are gone.
    let part = {
        let p = m.get(id).unwrap().path;
        let mut s = p.into_os_string();
        s.push(".reidm");
        PathBuf::from(s)
    };
    std::fs::remove_file(&part).unwrap();

    m.resume(id).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);

    let got = std::fs::read(m.get(id).unwrap().path).unwrap();
    assert!(got == data, "a lost part file must restart the download, not leave a hole");
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_on_a_range_server_is_not_corrupt() {
    let data = body(3_000_000);
    let srv = serve(data.clone(), true, 600_000);
    let dir = tmp("range");
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 8).unwrap();

    let id = m
        .add(Request { url: format!("http://127.0.0.1:{}/file.bin", srv.port), dir: Some(dir.join("dl")), ..Default::default() })
        .unwrap();

    wait_until("progress", || m.get(id).unwrap().downloaded > 100_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);
    m.resume(id).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);

    let got = std::fs::read(m.get(id).unwrap().path).unwrap();
    assert!(got == data, "segmented resume must be byte-identical");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_downloads_with_the_same_name_do_not_share_a_file() {
    let data = body(2_000_000);
    let srv = serve(data.clone(), true, 0);
    let dir = tmp("dupname");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let url = format!("http://127.0.0.1:{}/same.zip", srv.port);
    let a = m.add(Request { url: url.clone(), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    let b = m.add(Request { url, dir: Some(dl.clone()), ..Default::default() }).unwrap();

    let pa = m.get(a).unwrap().path;
    let pb = m.get(b).unwrap().path;
    assert_ne!(pa, pb, "two downloads must not be given the same path");

    wait_until("both done", || state(&m, a) == Status::Done && state(&m, b) == Status::Done);
    assert_eq!(std::fs::read(pa).unwrap(), data);
    assert_eq!(std::fs::read(pb).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_byte_file_downloads() {
    let srv = serve(Vec::new(), true, 0);
    let dir = tmp("empty");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let id = m.add(Request { url: format!("http://127.0.0.1:{}/empty.txt", srv.port), dir: Some(dl), ..Default::default() }).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);
    assert_eq!(std::fs::read(m.get(id).unwrap().path).unwrap(), Vec::<u8>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_unfinished_download_takes_its_part_file() {
    let data = body(3_000_000);
    let srv = serve(data, true, 400_000);
    let dir = tmp("partdrop");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let id = m.add(Request { url: format!("http://127.0.0.1:{}/partial.bin", srv.port), dir: Some(dl), ..Default::default() }).unwrap();
    wait_until("progress", || m.get(id).unwrap().downloaded > 50_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);

    let mut part = m.get(id).unwrap().path.into_os_string();
    part.push(".reidm");
    let part = PathBuf::from(part);
    assert!(part.exists());

    // Keeping the file, dropping the row: the scratch file must still go, or the next download
    // of the same name silently becomes "name (1).bin".
    m.remove(id, false);
    assert!(!part.exists(), "the part file must not be orphaned");
}

#[tokio::test(flavor = "multi_thread")]
async fn purge_removes_a_completed_file() {
    let data = body(500_000);
    let srv = serve(data.clone(), true, 0);
    let dir = tmp("purge");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let id = m.add(Request { url: format!("http://127.0.0.1:{}/gone.bin", srv.port), dir: Some(dl), ..Default::default() }).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);
    let path = m.get(id).unwrap().path;
    assert!(path.exists());

    m.remove(id, true);
    assert!(!path.exists(), "--purge must delete the finished file too");
    assert!(m.get(id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_list_and_resume_do_not_deadlock() {
    let data = body(400_000);
    let srv = serve(data.clone(), true, 0);
    let dir = tmp("deadlock");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let id = m.add(Request { url: format!("http://127.0.0.1:{}/x.bin", srv.port), dir: Some(dl), ..Default::default() }).unwrap();
    wait_until("done", || state(&m, id) == Status::Done);

    // `list` used to take live-then-db while `resume` took db-then-live; hammering both at once
    // would wedge the daemon permanently.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let m = m.clone();
        let stop = stop.clone();
        handles.push(std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = m.list();
            }
        }));
    }
    for _ in 0..4 {
        let m = m.clone();
        let stop = stop.clone();
        handles.push(std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = m.resume(id);
            }
        }));
    }
    std::thread::sleep(Duration::from_secs(3));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for h in handles {
        h.join().expect("a thread wedged — lock order is inconsistent");
    }
}
