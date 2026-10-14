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

/// The page the grabber tests scan: links to real files, a sub-page, an off-site link, a
/// non-file link, and a URL that only exists inside a script.
const INDEX_HTML: &str = r##"<!doctype html><html><head><base href="/files/"></head><body>
<a href="one.zip">one</a>
<a href="two.iso">two</a>
<a href="clip.mp4">clip</a>
<a href="page.html">a page, not a file</a>
<a href="/other.html">another page</a>
<a href="https://offsite.invalid/remote.deb">off site</a>
<script>var cfg = {"x": "inside-script.zip"};</script>
</body></html>"##;

const OTHER_HTML: &str = r##"<!doctype html><html><body>
<a href="/files/deep.tar.gz">deep</a>
</body></html>"##;

fn handle_conn(mut s: TcpStream, data: &[u8], ranges: bool, throttle: usize) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
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

        // HTML pages, for the grabber tests.
        if path.ends_with(".html") || path == "/" {
            let page = if path.contains("other") { OTHER_HTML } else { INDEX_HTML };
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n", page.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(page.as_bytes());
            let _ = s.flush();
            continue;
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

    // Different URLs, same file name: these must get different paths. (Adding one URL twice is
    // now a no-op — see `re_adding_a_url_does_not_make_a_second_copy`.)
    let a = m.add(Request { url: format!("http://127.0.0.1:{}/one/same.zip", srv.port), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    let b = m.add(Request { url: format!("http://127.0.0.1:{}/two/same.zip", srv.port), dir: Some(dl.clone()), ..Default::default() }).unwrap();

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

// ── batch, queueing and re-adding a link ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn re_adding_a_url_does_not_make_a_second_copy() {
    use reidm_core::{AddOutcome, Outcome};
    let data = body(2_000_000);
    let srv = serve(data.clone(), true, 0);
    let dir = tmp("dedupe");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();
    let url = format!("http://127.0.0.1:{}/file.bin", srv.port);

    let first: AddOutcome = m.add_detailed(Request { url: url.clone(), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    assert_eq!(first.outcome, Outcome::New);

    // Same file, different fragment and query order: still the same download.
    let second = m
        .add_detailed(Request { url: format!("{url}#again"), dir: Some(dl.clone()), ..Default::default() })
        .unwrap();
    assert_eq!(second.id, first.id, "the same URL must not create a second entry");
    assert_eq!(second.outcome, Outcome::Existing);

    wait_until("done", || state(&m, first.id) == Status::Done);
    assert_eq!(m.list().len(), 1, "only one entry should exist");

    let third = m.add_detailed(Request { url, dir: Some(dl), ..Default::default() }).unwrap();
    assert_eq!(third.id, first.id);
    assert_eq!(third.outcome, Outcome::AlreadyComplete, "a finished file should be recognised");
}

#[tokio::test(flavor = "multi_thread")]
async fn re_adding_a_stopped_url_resumes_it() {
    let data = body(3_000_000);
    let srv = serve(data.clone(), true, 400_000); // slow enough to interrupt
    let dir = tmp("readd");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();
    let url = format!("http://127.0.0.1:{}/resume-me.bin", srv.port);

    let id = m.add(Request { url: url.clone(), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("progress", || m.get(id).unwrap().downloaded > 50_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);

    // Pasting the link again should carry on, not start a fresh copy beside it.
    let again = m.add_detailed(Request { url, dir: Some(dl), ..Default::default() }).unwrap();
    assert_eq!(again.id, id);
    assert_eq!(again.outcome, reidm_core::Outcome::Existing);
    wait_until("done", || state(&m, id) == Status::Done);
    assert_eq!(m.list().len(), 1);
    assert_eq!(std::fs::read(m.get(id).unwrap().path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrency_limit_queues_the_rest() {
    let data = body(3_000_000);
    let srv = serve(data.clone(), true, 500_000);
    let dir = tmp("queue");
    let dl = dir.join("dl");
    let m = Manager::open_with(&dir.join("state"), dl.clone(), 2, 2).unwrap();

    let mut ids = Vec::new();
    for i in 0..6 {
        let url = format!("http://127.0.0.1:{}/q{i}.bin", srv.port);
        ids.push(m.add(Request { url, dir: Some(dl.clone()), ..Default::default() }).unwrap());
    }

    std::thread::sleep(Duration::from_millis(1200));
    let running = m.list().iter().filter(|i| i.status == Status::Running).count();
    assert!(running <= 2, "at most 2 should transfer at once, saw {running}");

    // Everything must finish: the queue drains as slots free up.
    wait_until("all done", || m.list().iter().all(|i| i.status == Status::Done));
    assert_eq!(m.list().len(), 6);
    for id in ids {
        assert_eq!(std::fs::read(m.get(id).unwrap().path).unwrap(), data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_add_skips_duplicates_and_reports_each() {
    use reidm_core::Outcome;
    let data = body(500_000);
    let srv = serve(data, true, 0);
    let dir = tmp("batch");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("state"), dl.clone(), 4).unwrap();

    let mk = |n: &str| Request { url: format!("http://127.0.0.1:{}/{n}.bin", srv.port), dir: Some(dl.clone()), ..Default::default() };
    let r = m.add_many(vec![mk("a"), mk("b"), mk("a"), mk("c"), mk("b")]);

    assert_eq!(r.len(), 5);
    assert_eq!(r[0].1.as_ref().unwrap().outcome, Outcome::New);
    assert_eq!(r[1].1.as_ref().unwrap().outcome, Outcome::New);
    assert_eq!(r[2].1.as_ref().unwrap().outcome, Outcome::Existing, "duplicate in the same batch");
    assert_eq!(r[4].1.as_ref().unwrap().outcome, Outcome::Existing);
    assert_eq!(m.list().len(), 3, "three distinct URLs");
}

// ── site grabber ────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn grabber_finds_files_and_skips_non_files() {
    use reidm_core::grabber::{self, Options};
    let srv = serve(body(1000), true, 0);
    let base = format!("http://127.0.0.1:{}", srv.port);

    let opts = Options {
        types: grabber::COMMON_TYPES.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let found = grabber::grab_page(&format!("{base}/index.html"), &opts).await.unwrap();
    let urls: Vec<&str> = found.iter().map(|f| f.url.as_str()).collect();

    // The <base href="/files/"> must be honoured, so these resolve under /files/.
    assert!(urls.contains(&format!("{base}/files/one.zip").as_str()), "got {urls:?}");
    assert!(urls.contains(&format!("{base}/files/two.iso").as_str()));
    assert!(urls.contains(&format!("{base}/files/clip.mp4").as_str()));
    assert!(urls.contains(&"https://offsite.invalid/remote.deb"), "absolute links keep their host");
    // Things that are not files must not be offered.
    assert!(!urls.iter().any(|u| u.ends_with("page.html")), "an html page is not a download");
    assert!(!urls.iter().any(|u| u.contains("inside-script")), "URLs inside <script> are not links");
}

#[tokio::test(flavor = "multi_thread")]
async fn grabber_follows_links_one_level() {
    use reidm_core::grabber::{self, Options};
    let srv = serve(body(1000), true, 0);
    let base = format!("http://127.0.0.1:{}", srv.port);

    // At depth 0 the page's own files are found, but nothing behind its links.
    let shallow = Options::default();
    let only_this_page = grabber::grab_page(&format!("{base}/index.html"), &shallow).await.unwrap();
    let shallow_urls: Vec<&str> = only_this_page.iter().map(|f| f.url.as_str()).collect();
    assert!(shallow_urls.contains(&format!("{base}/files/one.zip").as_str()));
    assert!(
        !shallow_urls.iter().any(|u| u.contains("deep.tar.gz")),
        "depth 0 must not follow links: {shallow_urls:?}"
    );

    let deep = Options { depth: 1, ..Default::default() };
    let found = grabber::grab_page(&format!("{base}/index.html"), &deep).await.unwrap();
    let urls: Vec<&str> = found.iter().map(|f| f.url.as_str()).collect();
    assert!(
        urls.contains(&format!("{base}/files/deep.tar.gz").as_str()),
        "a file linked from a followed page should be found: {urls:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn grabber_can_report_sizes() {
    use reidm_core::grabber::{self, Options};
    let data = body(4096);
    let srv = serve(data.clone(), true, 0);
    let base = format!("http://127.0.0.1:{}", srv.port);

    let opts = Options { types: vec!["zip".into()], sizes: true, ..Default::default() };
    let found = grabber::grab_page(&format!("{base}/index.html"), &opts).await.unwrap();
    assert!(!found.is_empty());
    for f in &found {
        assert_eq!(f.size, Some(data.len() as u64), "size should come from Content-Length");
    }
}
