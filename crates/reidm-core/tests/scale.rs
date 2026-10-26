//! How the list behaves when it is large: adding thousands of links at once must not rewrite
//! the whole list once per link, and must not hold the service up while it does.

use reidm_core::{AddOptions, Manager, Request};
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
    let p = std::env::temp_dir().join(format!("reidm-scale-{tag}-{}", std::process::id()));
    AutoCleanDir::create(p)
}

/// Links that are never fetched: `later` keeps them in the list without starting them.
fn links(n: usize, dl: &std::path::Path, tag: &str) -> Vec<Request> {
    (0..n)
        .map(|i| Request {
            url: format!("http://127.0.0.1:9/{tag}/file-{i}.bin?token=abc{i}"),
            cookie: Some("session=0123456789abcdef0123456789abcdef".into()),
            dir: Some(dl.to_path_buf()),
            ..Default::default()
        })
        .collect()
}

/// The time to add `n` links to a list that already has `n`, for the before/after numbers in
/// the decision log: `REIDM_SCALE_N=10000 cargo test --release --test scale -- --ignored`.
#[test]
#[ignore]
fn measure_batch_add() {
    let n: usize = std::env::var("REIDM_SCALE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
    let dir = tmp("measure");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("data"), dl.clone(), 4).unwrap();
    let t = Instant::now();
    m.add_many_with(links(n, &dl, "a"), AddOptions { later: true, ..Default::default() });
    let first = t.elapsed();
    let t = Instant::now();
    m.add_many_with(links(n, &dl, "b"), AddOptions { later: true, ..Default::default() });
    let second = t.elapsed();
    let t = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(m.list());
    }
    let list = t.elapsed() / 100;
    eprintln!("add_many {n} into empty: {first:?}; {n} more into {n}: {second:?}; list() of {}: {list:?}", 2 * n);
}

#[test]
fn adding_thousands_of_links_is_quick() {
    let n = 5_000;
    let dir = tmp("batch");
    let dl = dir.join("dl");
    let m = Manager::open(&dir.join("data"), dl.clone(), 4).unwrap();
    let t = Instant::now();
    let out = m.add_many_with(links(n, &dl, "x"), AddOptions { later: true, ..Default::default() });
    let took = t.elapsed();
    assert_eq!(out.len(), n);
    assert!(out.iter().all(|(_, r)| r.is_ok()));
    // Generous for a debug build on a busy runner. Saving the list once per link made this
    // take minutes, because each save rewrote every link added before it.
    assert!(took < Duration::from_secs(20), "adding {n} links took {took:?}");

    // The same links again are all recognised as already listed, just as quickly.
    let t = Instant::now();
    let again = m.add_many_with(links(n, &dl, "x"), AddOptions { later: true, ..Default::default() });
    assert!(t.elapsed() < Duration::from_secs(20));
    assert!(again.iter().all(|(_, r)| matches!(r, Ok(o) if o.outcome == reidm_core::Outcome::Existing)));
    assert_eq!(m.list().len(), n);
}
