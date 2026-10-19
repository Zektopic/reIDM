//! Torrents, end to end and offline: a second torrent client on this machine creates and seeds
//! a torrent, and reIDM downloads it from that one peer — no trackers, no DHT — then seeds it,
//! stops seeding, and removes it with its files.

use librqbit::{AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session, SessionOptions};
use reidm_core::{Manager, Request, SettingsPatch, Status, TorrentRequest};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reidm-torrent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
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
