//! Torrents, end to end and offline: a second torrent client on this machine creates and seeds
//! a torrent, and reIDM downloads it from that one peer — no trackers, no DHT — then seeds it,
//! stops seeding, and removes it with its files.

use librqbit::{AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session, SessionOptions};
use reidm_core::{Manager, Request, SettingsPatch, Status, TorrentRequest};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[allow(dead_code)]
struct AutoCleanDir {
    path: PathBuf,
}

#[allow(dead_code)]
impl AutoCleanDir {
    fn create(path: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn new(path: PathBuf) -> Self {
        Self::create(path)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for AutoCleanDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for AutoCleanDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for AutoCleanDir {
    fn drop(&mut self) {
        if matches!(std::env::var("REIDM_KEEP_TEST_DIR").as_deref(), Ok("1" | "true")) {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.path);
        if self.path.exists() {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(50));
                let _ = std::fs::remove_dir_all(&self.path);
                if !self.path.exists() {
                    break;
                }
            }
        }
    }
}

fn tmp(tag: &str) -> AutoCleanDir {
    let p = std::env::temp_dir().join(format!("reidm-torrent-{tag}-{}", std::process::id()));
    AutoCleanDir::create(p)
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_downloads_from_a_peer_seeds_and_is_removed_with_its_files() {
    let dir = tmp("e2e");
    // The other client: a file, a torrent of it, and that torrent seeded on localhost.
    let seed_dir = dir.join("seeder");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let data: Vec<u8> = (0..2_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    std::fs::write(seed_dir.join("payload.bin"), &data).unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(4);
    let created = librqbit::create_torrent(&seed_dir.join("payload.bin"), CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(65536) }, &spawner).await.unwrap();
    let bytes = created.as_bytes().unwrap();
    let hash = created.info_hash().as_string();
    // A free port on localhost (librqbit announces no port for a loopback address).
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let seeder = Session::new_with_opts(
        seed_dir.clone(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            listen: Some(ListenerOptions { listen_addr: (std::net::Ipv4Addr::LOCALHOST, port).into(), ..Default::default() }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let h = seeder
        .add_torrent(AddTorrent::from_bytes(bytes.clone()), Some(AddTorrentOptions { output_folder: Some(seed_dir.to_string_lossy().into()), overwrite: true, ..Default::default() }))
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    wait_for("the seeder to check its file", || h.stats().finished).await;

    // reIDM, kept off the network: no DHT, any port.
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();
    m.update_settings(SettingsPatch { torrent_dht: Some(false), torrent_port: Some(0), ..Default::default() }).unwrap();
    let file = dir.join("t.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let id = m
        .add(Request {
            url: format!("magnet:?xt=urn:btih:{hash}"),
            torrent: Some(TorrentRequest { file: Some(file), peers: vec![format!("127.0.0.1:{port}")], ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    wait_for("the torrent to finish", || matches!(m.get(id).unwrap().status, Status::Done | Status::Failed(_))).await;
    let it = m.get(id).unwrap();
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    assert_eq!(std::fs::read(&it.path).unwrap(), data, "the bytes are the seeder's");
    assert_eq!(it.path.parent().unwrap(), dir.join("dl"), "a .bin is filed with everything else");
    let t = it.torrent.clone().unwrap();
    assert_eq!(t.info_hash, hash);
    assert_eq!((t.files.len(), t.files[0].size, t.files[0].done), (1, 2_000_000, 2_000_000));
    assert!(t.seeding, "a finished torrent goes on sharing (share ratio 1.0 by default)");

    m.set_seeding(id, false).await.unwrap();
    assert!(!m.get(id).unwrap().torrent.unwrap().seeding);

    m.remove(id, true);
    wait_for("its file to go", || !it.path.exists()).await;
}

#[test]
fn magnet_links_and_torrent_addresses_become_torrents() {
    let dir = tmp("add");
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();
    m.update_settings(SettingsPatch { sort_by_category: Some(true), ..Default::default() }).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _g = rt.enter();
    let a = m
        .add_with(Request { url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Movie.mkv".into(), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() })
        .unwrap();
    let it = m.get(a.id).unwrap();
    assert!(it.request.torrent.is_some());
    assert_eq!(it.path, dir.join("dl").join("Video").join("Movie.mkv"), "named and filed by the magnet's dn");
    // The same torrent with other trackers is the same download.
    let again = m
        .add_with(Request { url: "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&tr=udp://t.example:1".into(), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() })
        .unwrap();
    assert_eq!(again.id, a.id);
    // Not every scheme: only magnet links besides web addresses.
    assert!(m.add(Request { url: "ftp://example.org/x.zip".into(), ..Default::default() }).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_file_on_the_web_is_added_as_a_torrent_with_the_browsers_cookies() {
    use std::io::{Read, Write};
    let dir = tmp("web");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src").join("movie.mkv"), vec![7u8; 100_000]).unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = librqbit::create_torrent(&dir.join("src").join("movie.mkv"), CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(16384) }, &spawner).await.unwrap();
    let bytes = created.as_bytes().unwrap().to_vec();
    let hash = created.info_hash().as_string();

    // A site that hands the .torrent only to a signed-in browser, from an address that doesn't
    // say it is one; the name comes from the browser.
    let srv = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = srv.local_addr().unwrap().port();
    let served = bytes.clone();
    std::thread::spawn(move || {
        for s in srv.incoming() {
            let mut s = s.unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            if req.contains("cookie: session=ok") {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/x-bittorrent\r\nContent-Length: {}\r\n\r\n", served.len());
                let _ = s.write_all(&served);
            } else {
                let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
            }
        }
    });

    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();
    m.update_settings(SettingsPatch { torrent_dht: Some(false), torrent_port: Some(0), ..Default::default() }).unwrap();
    let r = m
        .add_checked(
            Request { url: format!("http://127.0.0.1:{port}/download.php?id=9"), filename: Some("movie.mkv.torrent".into()), cookie: Some("session=ok".into()), ..Default::default() },
            reidm_core::AddOptions { later: true, ..Default::default() },
        )
        .await
        .unwrap();
    let it = m.get(r.id).unwrap();
    assert_eq!(it.request.url, format!("magnet:?xt=urn:btih:{hash}&dn=movie%2Emkv"), "known by its info hash");
    let kept = it.request.torrent.unwrap().file.unwrap();
    assert_eq!(std::fs::read(kept).unwrap(), bytes, "the .torrent is kept in the data folder");
    assert_eq!(it.path.parent().unwrap(), dir.join("dl").join("Video"), "and filed by what it holds");
}

/// Another client on this machine seeding `what` (a file, or a folder of files): the torrent's
/// bytes, its info hash, and the `ip:port` to reach it at. The session is returned so it lives
/// as long as the test.
async fn seed(what: &std::path::Path) -> (Vec<u8>, String, String, std::sync::Arc<Session>) {
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = librqbit::create_torrent(what, CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(16384) }, &spawner).await.unwrap();
    let bytes = created.as_bytes().unwrap().to_vec();
    let hash = created.info_hash().as_string();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let seeder = Session::new_with_opts(
        what.parent().unwrap().to_path_buf(),
        SessionOptions { dht: None, disable_trackers: true, listen: Some(ListenerOptions { listen_addr: (std::net::Ipv4Addr::LOCALHOST, port).into(), ..Default::default() }), ..Default::default() },
    )
    .await
    .unwrap();
    // A folder's files are inside it; a single file is beside the others.
    let out = if what.is_dir() { what.to_path_buf() } else { what.parent().unwrap().to_path_buf() };
    let h = seeder
        .add_torrent(AddTorrent::from_bytes(bytes.clone()), Some(AddTorrentOptions { output_folder: Some(out.to_string_lossy().into()), overwrite: true, ..Default::default() }))
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    wait_for("the seeder to check its files", || h.stats().finished).await;
    (bytes, hash, format!("127.0.0.1:{port}"), seeder)
}

fn quiet_manager(dir: &std::path::Path) -> Manager {
    let m = Manager::open(&dir.join("state"), dir.join("dl"), 4).unwrap();
    m.update_settings(SettingsPatch { torrent_dht: Some(false), torrent_port: Some(0), seed_ratio: Some(0.0), ..Default::default() }).unwrap();
    m
}

fn from_peer(dir: &std::path::Path, tag: &str, bytes: &[u8], hash: &str, peer: &str) -> Request {
    let file = dir.join(format!("{tag}.torrent"));
    std::fs::write(&file, bytes).unwrap();
    Request { url: format!("magnet:?xt=urn:btih:{hash}"), torrent: Some(TorrentRequest { file: Some(file), peers: vec![peer.to_string()], ..Default::default() }), ..Default::default() }
}

async fn finished(m: &Manager, id: u64) -> reidm_core::Item {
    wait_for("the torrent to end", || matches!(m.get(id).unwrap().status, Status::Done | Status::Failed(_))).await;
    m.get(id).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_torrent_never_writes_over_a_file_that_is_already_there() {
    let dir = tmp("collide");
    std::fs::create_dir_all(dir.join("seeder")).unwrap();
    let data = vec![42u8; 300_000];
    std::fs::write(dir.join("seeder").join("payload.bin"), &data).unwrap();
    let (bytes, hash, peer, _seeder) = seed(&dir.join("seeder").join("payload.bin")).await;

    // The user's own, different, file of the same name, where the torrent would go.
    std::fs::create_dir_all(dir.join("dl")).unwrap();
    std::fs::write(dir.join("dl").join("payload.bin"), b"mine, not the torrent's").unwrap();
    let m = quiet_manager(&dir);
    let id = m.add(from_peer(&dir, "t", &bytes, &hash, &peer)).unwrap();
    let it = finished(&m, id).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    assert_eq!(std::fs::read(dir.join("dl").join("payload.bin")).unwrap(), b"mine, not the torrent's", "the user's file is untouched");
    assert_eq!(it.path, dir.join("dl").join("payload").join("payload.bin"), "the torrent went into a folder of its own");
    assert_eq!(std::fs::read(&it.path).unwrap(), data);
    assert_eq!(it.torrent.unwrap().folder, Some(dir.join("dl").join("payload")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_of_several_files_gets_a_folder_of_its_own() {
    let dir = tmp("multi");
    let album = dir.join("seeder").join("Album");
    std::fs::create_dir_all(&album).unwrap();
    std::fs::write(album.join("one.flac"), vec![1u8; 70_000]).unwrap();
    std::fs::write(album.join("two.flac"), vec![2u8; 50_000]).unwrap();
    let (bytes, hash, peer, _seeder) = seed(&album).await;

    // A folder of that name is already there, with a file the torrent also has.
    std::fs::create_dir_all(dir.join("dl").join("Album")).unwrap();
    std::fs::write(dir.join("dl").join("Album").join("one.flac"), b"another album").unwrap();
    let m = quiet_manager(&dir);
    let id = m.add(from_peer(&dir, "t", &bytes, &hash, &peer)).unwrap();
    let it = finished(&m, id).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    let folder = dir.join("dl").join("Album (1)");
    assert_eq!(it.path, folder, "the list shows the torrent's folder");
    assert_eq!(std::fs::read(folder.join("one.flac")).unwrap(), vec![1u8; 70_000]);
    assert_eq!(std::fs::read(folder.join("two.flac")).unwrap(), vec![2u8; 50_000]);
    assert_eq!(std::fs::read(dir.join("dl").join("Album").join("one.flac")).unwrap(), b"another album");
    assert!(!dir.join("dl").join("two.flac").exists(), "nothing is scattered into the download folder");

    // Removed with its files: the folder goes, the other album stays.
    m.remove(id, true);
    wait_for("its files to go", || !folder.join("one.flac").exists()).await;
    assert!(dir.join("dl").join("Album").join("one.flac").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_magnet_link_gets_its_description_from_a_peer() {
    let dir = tmp("magnet");
    std::fs::create_dir_all(dir.join("seeder")).unwrap();
    let data: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("seeder").join("payload.bin"), &data).unwrap();
    let (_bytes, hash, peer, _seeder) = seed(&dir.join("seeder").join("payload.bin")).await;

    // No .torrent is given: the name, the files and their sizes have to come from a peer, as
    // they must for every magnet link. This is the path whose two-minute bound failed a real
    // magnet in a quiet swarm.
    let m = quiet_manager(&dir);
    let id = m
        .add(Request {
            url: format!("magnet:?xt=urn:btih:{hash}&dn=payload.bin"),
            torrent: Some(TorrentRequest { peers: vec![peer], ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    let it = finished(&m, id).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    assert_eq!(std::fs::read(&it.path).unwrap(), data, "the bytes are the seeder's");
    let t = it.torrent.clone().unwrap();
    assert_eq!(t.info_hash, hash);
    assert_eq!(t.name, "payload.bin");
    assert_eq!(t.files.len(), 1);
    // The description the peer gave is kept, so starting it again need not ask the swarm.
    let kept = it.request.torrent.as_ref().and_then(|r| r.file.clone()).expect("the .torrent is kept");
    let kept_bytes = std::fs::read(kept).unwrap();
    assert_eq!(librqbit::torrent_from_bytes(&kept_bytes).unwrap().info_hash.as_string(), hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_a_finished_torrent_again_downloads_it_again() {
    let dir = tmp("restart");
    std::fs::create_dir_all(dir.join("seeder")).unwrap();
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(dir.join("seeder").join("disc.iso"), &data).unwrap();
    let (bytes, hash, peer, _seeder) = seed(&dir.join("seeder").join("disc.iso")).await;
    let m = quiet_manager(&dir);
    let id = m.add(from_peer(&dir, "t", &bytes, &hash, &peer)).unwrap();
    let first = finished(&m, id).await;
    assert_eq!(first.status, Status::Done);
    // "Start again": its files are deleted and it is fetched anew. The deletion happens in the
    // background; the new run must wait for it rather than pick up the torrent being removed
    // (whose files would then be deleted from under it), or find its old file and step aside.
    m.restart(id).await.unwrap();
    let again = finished(&m, id).await;
    assert_eq!(again.status, Status::Done, "{:?}", again.status);
    assert_eq!(again.path, first.path, "the same place, not a folder beside it");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(std::fs::read(&again.path).unwrap(), data, "and its file stays");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_torrents_files_are_deleted_after_a_restart() {
    let dir = tmp("restart-delete");
    std::fs::create_dir_all(dir.join("seeder")).unwrap();
    let data = vec![3u8; 120_000];
    std::fs::write(dir.join("seeder").join("payload.bin"), &data).unwrap();
    let (bytes, hash, peer, _seeder) = seed(&dir.join("seeder").join("payload.bin")).await;
    let id;
    {
        let m = quiet_manager(&dir);
        id = m.add(from_peer(&dir, "t", &bytes, &hash, &peer)).unwrap();
        assert_eq!(finished(&m, id).await.status, Status::Done);
    }
    let it = {
        let m = quiet_manager(&dir);
        m.get(id).unwrap()
    };
    let file = it.path.clone();
    assert!(file.exists());
    // A new manager has no torrent session: there is no engine handle to ask, so the files
    // recorded in the list have to go directly.
    let m = quiet_manager(&dir);
    m.remove(id, true);
    wait_for("its file to go", || !file.exists()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_torrent_is_one_download_whatever_form_its_link_takes() {
    let dir = tmp("twins");
    std::fs::create_dir_all(dir.join("seeder")).unwrap();
    std::fs::write(dir.join("seeder").join("song.mp3"), vec![9u8; 40_000]).unwrap();
    let (bytes, hash, peer, _seeder) = seed(&dir.join("seeder").join("song.mp3")).await;
    let m = quiet_manager(&dir);
    let later = || reidm_core::AddOptions { later: true, ..Default::default() };
    let a = m.add_with(from_peer(&dir, "first", &bytes, &hash, &peer), later()).unwrap();
    // The same info hash in base32, as some sites write it, and the .torrent itself.
    let b32 = base32(&hex(&hash));
    let b = m.add_with(Request { url: format!("magnet:?xt=urn:btih:{b32}&dn=song.mp3"), ..Default::default() }, later()).unwrap();
    assert_eq!(b.id, a.id, "hex and base32 name the same torrent");
    assert_eq!(m.add_torrent_file(&bytes, None, true).unwrap().id, a.id);

    // Under an address the list can't tell is the same: refused when it starts, not run twice.
    m.resume(a.id).unwrap();
    let first = finished(&m, a.id).await;
    assert_eq!(first.status, Status::Done, "{:?}", first.status);
    let mut other = from_peer(&dir, "other", &bytes, &hash, &peer);
    other.url = "https://example.invalid/song.torrent".into();
    let c = m.add(other).unwrap();
    assert_ne!(c, a.id);
    match finished(&m, c).await.status {
        Status::Failed(e) => assert!(e.contains(&format!("already in the list (download {})", a.id)), "{e}"),
        s => panic!("ran a second time: {s:?}"),
    }
    assert!(first.path.exists(), "the first one's file is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_address_that_sends_without_end_is_refused() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = tmp("endless");
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let count = sent.clone();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut req = [0u8; 4096];
        let _ = s.read(&mut req).await;
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-bittorrent\r\nTransfer-Encoding: chunked\r\n\r\n").await;
        let chunk = format!("{:x}\r\n{}\r\n", 1 << 20, "d".repeat(1 << 20));
        // Up to 1 GiB, unless the reader hangs up first.
        for _ in 0..1024 {
            if s.write_all(chunk.as_bytes()).await.is_err() {
                return;
            }
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let m = quiet_manager(&dir);
    let e = m.add_checked(Request { url: format!("http://{addr}/big.torrent"), ..Default::default() }, Default::default()).await.unwrap_err();
    assert!(format!("{e:#}").contains("larger than 10 MiB"), "{e:#}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sent.load(std::sync::atomic::Ordering::SeqCst) < 64, "stopped reading at the limit, not at the end");
}

fn hex(h: &str) -> Vec<u8> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

fn base32(b: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut bits, mut n) = (String::new(), 0u32, 0);
    for &x in b {
        bits = (bits << 8) | x as u32;
        n += 8;
        while n >= 5 {
            n -= 5;
            out.push(A[((bits >> n) & 31) as usize] as char);
        }
    }
    out
}
