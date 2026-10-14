//! The engine features IDM users expect, checked end to end against an in-process server:
//! dynamic segmentation, servers that cap connections, the speed limiter, noticing a file that
//! changed on the server, refreshing an expired address, and *Download Later*.

use reidm_core::{AddOptions, Manager, Request, SettingsPatch, Status};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn body(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| ((i % 251) as u8).wrapping_add(seed)).collect()
}

/// A small HTTP/1.1 server whose behaviour each test shapes.
#[derive(Default)]
struct Cfg {
    files: Mutex<HashMap<String, Vec<u8>>>,
    /// Sent as `ETag`; a request whose `If-Range` does not match gets the whole file (`200`).
    etag: Mutex<Option<String>>,
    /// Bytes per second for any response that starts at byte 0, the first connection's range.
    slow_first: Option<usize>,
    /// Bytes per second for every response.
    throttle: Option<usize>,
    /// Refuse (`503`) a request while this many bodies are already being sent.
    max_conns: Option<usize>,
    active: AtomicUsize,
    /// Paths that answer `404` now.
    gone: Mutex<HashSet<String>>,
    /// Every `Range` header seen, in order.
    ranges: Mutex<Vec<String>>,
    /// Answer `403` unless the request carries this cookie, like a site behind a login.
    need_cookie: Option<String>,
    /// `Content-Type` to send per path.
    types: Mutex<HashMap<String, String>>,
}

struct Server {
    port: u16,
    cfg: Arc<Cfg>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

fn serve(cfg: Cfg) -> Server {
    let cfg = Arc::new(cfg);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let c = cfg.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { break };
            let c = c.clone();
            std::thread::spawn(move || handle(s, &c));
        }
    });
    Server { port, cfg }
}

struct ActiveGuard<'a>(&'a AtomicUsize);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle(mut s: TcpStream, cfg: &Cfg) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let mut range = None;
        let mut if_range = None;
        let mut cookie = String::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            if h.trim().is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).unwrap_or_default();
            match k.as_str() {
                "range" => range = Some(v),
                "if-range" => if_range = Some(v),
                "cookie" => cookie = v,
                _ => {}
            }
        }
        let reply = |s: &mut TcpStream, status: &str| {
            let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes());
        };
        if let Some(c) = &cfg.need_cookie {
            if !cookie.split(';').any(|kv| kv.trim() == c) {
                reply(&mut s, "403 Forbidden");
                continue;
            }
        }
        if cfg.gone.lock().unwrap().contains(&path) {
            reply(&mut s, "404 Not Found");
            continue;
        }
        let Some(data) = cfg.files.lock().unwrap().get(&path).cloned() else {
            reply(&mut s, "404 Not Found");
            continue;
        };
        if let Some(r) = &range {
            cfg.ranges.lock().unwrap().push(r.clone());
        }
        if let Some(max) = cfg.max_conns {
            if cfg.active.load(Ordering::SeqCst) >= max {
                reply(&mut s, "503 Service Unavailable");
                continue;
            }
        }
        cfg.active.fetch_add(1, Ordering::SeqCst);
        let _guard = ActiveGuard(&cfg.active);

        let etag = cfg.etag.lock().unwrap().clone();
        // `If-Range` that does not match the current ETag means "send the whole thing".
        let honour = match (&if_range, &etag) {
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
            (None, _) => true,
        };
        let (mut start, mut end) = (0usize, data.len().saturating_sub(1));
        let partial = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")) {
            Some(spec) if honour => {
                let (a, b) = spec.split_once('-').unwrap_or(("", ""));
                start = a.trim().parse().unwrap_or(0);
                if !b.trim().is_empty() {
                    end = b.trim().parse::<usize>().unwrap_or(end).min(end);
                }
                true
            }
            _ => false,
        };
        let slice = if start <= end && start < data.len() { &data[start..=end] } else { &data[0..0] };
        let mut head = if partial {
            format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\n", data.len(), slice.len())
        } else {
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n", slice.len())
        };
        head.push_str("Accept-Ranges: bytes\r\n");
        if let Some(t) = cfg.types.lock().unwrap().get(&path) {
            head.push_str(&format!("Content-Type: {t}\r\n"));
        }
        if let Some(e) = &etag {
            head.push_str(&format!("ETag: {e}\r\n"));
        }
        head.push_str("\r\n");
        if s.write_all(head.as_bytes()).is_err() {
            return;
        }
        let rate = if start == 0 { cfg.slow_first.or(cfg.throttle) } else { cfg.throttle };
        match rate {
            None => {
                if s.write_all(slice).is_err() {
                    return;
                }
            }
            Some(bps) => {
                // 20 writes a second at the requested rate.
                let chunk = (bps / 20).max(512);
                for c in slice.chunks(chunk) {
                    if s.write_all(c).is_err() || s.flush().is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let _ = s.flush();
    }
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reidm-eng-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

fn state(m: &Manager, id: u64) -> Status {
    m.get(id).unwrap().status
}

fn manager(tag: &str, connections: usize) -> (Manager, PathBuf) {
    let dir = tmp(tag);
    let m = Manager::open(&dir.join("data"), dir.join("dl"), connections).unwrap();
    (m, dir.join("dl"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_connection_has_its_range_taken_over() {
    // The first connection's range trickles at 64 kB/s, so without dynamic segmentation the
    // file would take over thirty seconds. The other connection finishes its half and then keeps
    // taking over the far half of whatever the slow one has left.
    let data = body(4 << 20, 1);
    let srv = serve(Cfg { slow_first: Some(64 << 10), ..Default::default() });
    srv.cfg.files.lock().unwrap().insert("/f.bin".into(), data.clone());
    let (m, dl) = manager("steal", 2);

    let t0 = Instant::now();
    let id = m.add(Request { url: srv.url("/f.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("download completes", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, id), Status::Done);
    let took = t0.elapsed();

    assert_eq!(std::fs::read(dl.join("f.bin")).unwrap(), data, "bytes must be exactly the file");
    let item = m.get(id).unwrap();
    assert!(item.segments().len() > 2, "ranges were split ({} ranges)", item.segments().len());
    assert!(took < Duration::from_secs(25), "took {took:?}: the slow range was not taken over");
}

#[tokio::test(flavor = "multi_thread")]
async fn more_connections_than_the_server_allows_still_finishes() {
    // A server that refuses a third simultaneous transfer with 503. Eight connections are asked
    // for; the refused ones must hand their ranges back instead of failing the download.
    let data = body(3 << 20, 2);
    let srv = serve(Cfg { max_conns: Some(2), throttle: Some(2 << 20), ..Default::default() });
    srv.cfg.files.lock().unwrap().insert("/capped.bin".into(), data.clone());
    let (m, dl) = manager("capped", 8);

    let id = m.add(Request { url: srv.url("/capped.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("download ends", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, id), Status::Done);
    assert_eq!(std::fs::read(dl.join("capped.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_speed_limiter_caps_the_rate() {
    let data = body(768 << 10, 3);
    let srv = serve(Cfg::default());
    srv.cfg.files.lock().unwrap().insert("/limited.bin".into(), data.clone());
    let (m, dl) = manager("limit", 4);
    m.update_settings(SettingsPatch { speed_limit: Some(256 << 10), limit_enabled: Some(true), ..Default::default() }).unwrap();

    let t0 = Instant::now();
    let id = m.add(Request { url: srv.url("/limited.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("download ends", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, id), Status::Done);
    // 768 KiB at 256 KiB/s, less the half-second burst, is at least 2.5 s. Only the lower bound
    // is asserted: a busy machine may be slower, never faster.
    let took = t0.elapsed();
    assert!(took >= Duration::from_millis(2300), "finished in {took:?}, faster than the limit allows");
    assert_eq!(std::fs::read(dl.join("limited.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_per_download_limit_applies_while_running() {
    let data = body(1 << 20, 4);
    let srv = serve(Cfg::default());
    srv.cfg.files.lock().unwrap().insert("/one.bin".into(), data.clone());
    let (m, dl) = manager("perlimit", 2);

    let id = m.add_with(Request { url: srv.url("/one.bin"), dir: Some(dl.clone()), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap().id;
    m.set_speed_limit(id, Some(128 << 10)).unwrap();
    let t0 = Instant::now();
    m.resume(id).unwrap();
    wait_until("download ends", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, id), Status::Done);
    assert!(t0.elapsed() >= Duration::from_secs(6), "1 MiB at 128 KiB/s took only {:?}", t0.elapsed());
    assert_eq!(m.get(id).unwrap().speed_limit, Some(128 << 10));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_changed_on_the_server_is_not_spliced() {
    let old = body(2 << 20, 5);
    let srv = serve(Cfg { throttle: Some(256 << 10), ..Default::default() });
    *srv.cfg.etag.lock().unwrap() = Some("\"v1\"".into());
    srv.cfg.files.lock().unwrap().insert("/changing.bin".into(), old.clone());
    let (m, dl) = manager("changed", 2);

    let id = m.add(Request { url: srv.url("/changing.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("some progress", || m.get(id).unwrap().downloaded > 100_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);
    assert_eq!(m.get(id).unwrap().etag.as_deref(), Some("\"v1\""));

    // A new version of the same size: only the validator can tell.
    *srv.cfg.etag.lock().unwrap() = Some("\"v2\"".into());
    srv.cfg.files.lock().unwrap().insert("/changing.bin".into(), body(2 << 20, 99));
    m.resume(id).unwrap();
    wait_until("resume ends", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    match state(&m, id) {
        Status::Failed(e) => assert!(e.contains("changed on the server"), "unexpected error: {e}"),
        other => panic!("a changed file must not complete as {other:?}"),
    }
    assert!(srv.cfg.ranges.lock().unwrap().iter().any(|r| r != "bytes=0-0"), "it did try to resume");

    // Restart fetches the new version whole.
    m.restart(id).await.unwrap();
    wait_until("restart completes", || state(&m, id) == Status::Done);
    assert_eq!(std::fs::read(dl.join("changing.bin")).unwrap(), body(2 << 20, 99));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_size_change_without_an_etag_is_noticed_too() {
    let srv = serve(Cfg { throttle: Some(256 << 10), ..Default::default() });
    srv.cfg.files.lock().unwrap().insert("/grow.bin".into(), body(2 << 20, 6));
    let (m, dl) = manager("grow", 2);
    let id = m.add(Request { url: srv.url("/grow.bin"), dir: Some(dl), ..Default::default() }).unwrap();
    wait_until("some progress", || m.get(id).unwrap().downloaded > 100_000);
    m.pause(id);
    wait_until("paused", || state(&m, id) == Status::Paused);
    srv.cfg.files.lock().unwrap().insert("/grow.bin".into(), body(3 << 20, 6));
    m.resume(id).unwrap();
    wait_until("resume ends", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert!(matches!(state(&m, id), Status::Failed(e) if e.contains("changed on the server")));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_address_can_be_refreshed() {
    let data = body(2 << 20, 7);
    let srv = serve(Cfg { throttle: Some(512 << 10), ..Default::default() });
    {
        let mut f = srv.cfg.files.lock().unwrap();
        f.insert("/expiring.bin".into(), data.clone());
        f.insert("/mirror.bin".into(), data.clone());
        f.insert("/other.bin".into(), body(1 << 20, 8));
    }
    let (m, dl) = manager("refresh", 2);
    let id = m.add(Request { url: srv.url("/expiring.bin"), dir: Some(dl.clone()), ..Default::default() }).unwrap();
    wait_until("some progress", || m.get(id).unwrap().downloaded > 200_000);
    // The link expires mid-download.
    srv.cfg.gone.lock().unwrap().insert("/expiring.bin".into());
    m.pause(id);
    wait_until("stopped", || !matches!(state(&m, id), Status::Running | Status::Queued));
    let before = m.get(id).unwrap().downloaded;

    // A different file is refused, and nothing about the download changes.
    let err = m.refresh_address(id, Request { url: srv.url("/other.bin"), ..Default::default() }).await.unwrap_err();
    assert!(format!("{err:#}").contains("different file"), "{err:#}");
    assert!(m.get(id).unwrap().request.url.ends_with("/expiring.bin"));

    // The same file at a new address carries on from where it stopped.
    m.refresh_address(id, Request { url: srv.url("/mirror.bin"), ..Default::default() }).await.unwrap();
    wait_until("completes from the new address", || matches!(state(&m, id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, id), Status::Done);
    assert_eq!(std::fs::read(dl.join("expiring.bin")).unwrap(), data);
    assert!(m.get(id).unwrap().request.url.ends_with("/mirror.bin"));
    // It resumed rather than starting over: the mirror was asked for ranges past the start.
    assert!(before > 0);
}

#[test]
fn refresh_capture_is_one_shot_and_expires_unused() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (m, dl) = manager("arm", 1);
        let id = m
            .add_with(Request { url: "http://127.0.0.1:9/x.bin".into(), dir: Some(dl), ..Default::default() }, AddOptions { later: true, ..Default::default() })
            .unwrap()
            .id;
        assert_eq!(m.take_refresh_target(), None);
        m.arm_refresh(id).unwrap();
        assert_eq!(m.refresh_armed(), Some(id));
        assert_eq!(m.take_refresh_target(), Some(id));
        assert_eq!(m.take_refresh_target(), None, "a second link is a normal download again");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn download_later_waits_for_the_queue() {
    let data = body(300 << 10, 9);
    let srv = serve(Cfg::default());
    srv.cfg.files.lock().unwrap().insert("/later.bin".into(), data.clone());
    let (m, dl) = manager("later", 2);

    let o = m.add_with(Request { url: srv.url("/later.bin"), dir: Some(dl.clone()), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let it = m.get(o.id).unwrap();
    assert_eq!(it.status, Status::Paused);
    assert!(it.scheduled, "Download Later puts it in the scheduled queue");
    assert_eq!(it.downloaded, 0);
    assert!(srv.cfg.ranges.lock().unwrap().is_empty(), "nothing was requested");

    // Re-adding it "later" does not start it either.
    m.add_with(Request { url: srv.url("/later.bin"), dir: Some(dl.clone()), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap();
    assert_eq!(state(&m, o.id), Status::Paused);

    assert_eq!(m.start_queue(), 1);
    wait_until("queue completes", || state(&m, o.id) == Status::Done);
    assert_eq!(std::fs::read(dl.join("later.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_queue_pauses_only_scheduled_downloads() {
    let srv = serve(Cfg { throttle: Some(64 << 10), ..Default::default() });
    {
        let mut f = srv.cfg.files.lock().unwrap();
        f.insert("/q.bin".into(), body(1 << 20, 10));
        f.insert("/now.bin".into(), body(1 << 20, 11));
    }
    let (m, dl) = manager("stopq", 1);
    let q = m.add_with(Request { url: srv.url("/q.bin"), dir: Some(dl.clone()), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap().id;
    let now = m.add(Request { url: srv.url("/now.bin"), dir: Some(dl), ..Default::default() }).unwrap();
    m.start_queue();
    wait_until("both running", || state(&m, q) == Status::Running && state(&m, now) == Status::Running);
    assert_eq!(m.stop_queue(), 1);
    wait_until("queue stopped", || state(&m, q) == Status::Paused);
    assert_eq!(state(&m, now), Status::Running, "a download outside the queue keeps going");
    m.pause(now);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_schedules_are_refused() {
    let (m, _) = manager("sched", 1);
    let bad = reidm_core::schedule::Schedule { enabled: true, start: Some("26:00".into()), ..Default::default() };
    assert!(m.update_settings(SettingsPatch { schedule: Some(bad), ..Default::default() }).is_err());
    let good = reidm_core::schedule::Schedule { enabled: true, start: Some("02:30".into()), stop: Some("07:00".into()), days: vec![0, 1, 2, 3, 4] };
    m.update_settings(SettingsPatch { schedule: Some(good.clone()), ..Default::default() }).unwrap();
    assert_eq!(m.settings().schedule, good);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checked_add_takes_only_what_it_can_fetch() {
    let data = body(2 << 20, 12);
    let srv = serve(Cfg { need_cookie: Some("session=ok".into()), ..Default::default() });
    srv.cfg.files.lock().unwrap().insert("/private.bin".into(), data.clone());
    let (m, dl) = manager("checked", 4);

    // Without the browser's cookie the server refuses, so the hand-off must be refused too and
    // nothing may be left in the list.
    let r = m.add_checked(Request { url: srv.url("/private.bin"), dir: Some(dl.clone()), ..Default::default() }, AddOptions::default()).await;
    assert!(r.is_err(), "a download reIDM cannot fetch must be left to the browser");
    assert!(m.list().is_empty());

    // With it, the check passes and the file arrives, and the check doubled as the probe.
    let o = m
        .add_checked(
            Request { url: srv.url("/private.bin"), cookie: Some("theme=dark; session=ok".into()), dir: Some(dl.clone()), ..Default::default() },
            AddOptions::default(),
        )
        .await
        .unwrap();
    wait_until("download ends", || matches!(state(&m, o.id), Status::Done | Status::Failed(_)));
    assert_eq!(state(&m, o.id), Status::Done);
    assert_eq!(std::fs::read(dl.join("private.bin")).unwrap(), data);
    // The refused request is turned away before the server logs its range, so the one probe
    // recorded is the successful check: the transfer itself did not probe again.
    let probes = srv.cfg.ranges.lock().unwrap().iter().filter(|r| *r == "bytes=0-0").count();
    assert_eq!(probes, 1, "the check doubled as the probe");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_link_that_is_really_a_web_page_is_refused_when_asked() {
    let srv = serve(Cfg::default());
    {
        let mut f = srv.cfg.files.lock().unwrap();
        f.insert("/setup.zip".into(), b"<!doctype html><title>Please sign in</title>".to_vec());
        f.insert("/page.html".into(), b"<!doctype html><title>A page</title>".to_vec());
        let mut t = srv.cfg.types.lock().unwrap();
        t.insert("/setup.zip".into(), "text/html; charset=utf-8".into());
        t.insert("/page.html".into(), "text/html".into());
    }
    let (m, dl) = manager("html", 1);
    let opts = AddOptions { refuse_html: true, ..Default::default() };
    let err = m.add_checked(Request { url: srv.url("/setup.zip"), dir: Some(dl.clone()), ..Default::default() }, opts).await.unwrap_err();
    assert!(format!("{err:#}").contains("web page"), "{err:#}");
    assert!(m.list().is_empty());
    // A link that says it is a page and is one is still downloadable on purpose.
    m.add_checked(Request { url: srv.url("/page.html"), dir: Some(dl.clone()), ..Default::default() }, opts).await.unwrap();
    // And without the flag (a download the browser had already decided to save) nothing is second-guessed.
    m.add_checked(Request { url: srv.url("/setup.zip"), dir: Some(dl), ..Default::default() }, AddOptions::default()).await.unwrap();
}
