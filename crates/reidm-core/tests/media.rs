//! Video downloads through yt-dlp, against a stand-in for yt-dlp: a shell script that prints
//! what reIDM asks the real one to print (its progress and file lines), so these run offline.
//! Unix only, since the stand-in is a shell script.
#![cfg(unix)]

use reidm_core::{Manager, Media, Outcome, Request, Status};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FAKE_YTDLP: &str = r#"#!/bin/sh
# Stand-in for yt-dlp. Records its arguments; `mode` beside it says how to behave.
here=$(dirname "$0")
printf '%s\n' "$@" > "$here/args.txt"
out=""; home=""; temp=""; prev=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  [ "$prev" = "-P" ] && case "$a" in home:*) home="${a#home:}" ;; temp:*) temp="${a#temp:}" ;; esac
  prev="$a"
done
[ -n "$home" ] && out="$home/$out"
printf '%s' "$temp" > "$here/temp.txt"
file=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/; s/%(title)\.150B/Clip/; s/%(id)s/abc/; s/%(height&{}p|audio)s/360p/')
case "$(cat "$here/mode" 2>/dev/null)" in
  fail) echo "WARNING: [youtube] something minor" >&2; echo "ERROR: [youtube] abc: Video unavailable" >&2; exit 1 ;;
  slow) echo "REIDM-NAME $file"; echo "REIDM-SIZE 650000.0"
        i=0; while [ $i -lt 600 ]; do echo "REIDM-PROGRESS $((i * 1000)) 600000 NA NA 18"; i=$((i + 1)); sleep 0.05; done ;;
esac
echo "REIDM-NAME $file"
echo "REIDM-PROGRESS 500 1000 NA NA 137"
echo "REIDM-PROGRESS 1000 1000 NA NA 137"
echo "REIDM-PROGRESS 200 200 NA NA 140"
printf 'video' > "$file"
echo "REIDM-FILE $file"
"#;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reidm-media-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A manager whose tools folder holds the stand-ins, marked as just checked so nothing is
/// fetched from the network.
fn setup(tag: &str, mode: &str) -> (Manager, PathBuf, PathBuf) {
    let dir = tmp(tag);
    let state = dir.join("state");
    let tools = state.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    exe(&tools.join("yt-dlp"), FAKE_YTDLP);
    std::fs::write(tools.join("yt-dlp.version"), "test").unwrap();
    exe(&tools.join("deno"), "#!/bin/sh\n");
    std::fs::write(tools.join("mode"), mode).unwrap();
    let dl = dir.join("dl");
    let m = Manager::open(&state, dl.clone(), 4).unwrap();
    (m, dl, tools)
}

fn video(dl: &Path, format: &str) -> Request {
    Request {
        url: "https://www.youtube.com/watch?v=abc".into(),
        dir: Some(dl.to_path_buf()),
        media: Some(Media { format: format.into(), audio: None }),
        ..Default::default()
    }
}

async fn wait_for(m: &Manager, id: u64, what: impl Fn(&Status) -> bool) -> reidm_core::Item {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let it = m.get(id).unwrap();
        if what(&it.status) {
            return it;
        }
        assert!(Instant::now() < end, "still {:?}", it.status);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_is_fetched_by_ytdlp_and_named_after_its_title() {
    let (m, dl, tools) = setup("ok", "");
    let id = m.add(video(&dl, "137+bestaudio/best[height<=1080]")).unwrap();
    let it = wait_for(&m, id, |s| *s == Status::Done).await;
    assert_eq!(it.path, dl.join("Clip [abc] 360p.mp4"), "the name yt-dlp reported");
    assert_eq!(std::fs::read(&it.path).unwrap(), b"video");
    assert_eq!((it.size, it.downloaded), (Some(5), 5), "the finished file's size, not the progress estimate");

    let args: Vec<String> = std::fs::read_to_string(tools.join("args.txt")).unwrap().lines().map(String::from).collect();
    let after = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    assert_eq!(after("-f").as_deref(), Some("137+bestaudio/best[height<=1080]"));
    assert_eq!(after("--js-runtimes"), Some(format!("deno:{}", tools.join("deno").display())));
    assert_eq!(args[args.len() - 2..], ["--".to_string(), "https://www.youtube.com/watch?v=abc".to_string()], "the page last, after `--`");
    assert!(args.iter().any(|a| a == "--no-playlist"));
    let temp = std::fs::read_to_string(tools.join("temp.txt")).unwrap();
    assert!(temp.ends_with(&format!("parts/video-{id}")), "yt-dlp's pieces go to the temporary folder, not {temp:?}");
    assert!(!Path::new(&temp).exists(), "and that folder goes once the file is done");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_says_what_ytdlp_said() {
    let (m, dl, _) = setup("fail", "fail");
    let id = m.add(video(&dl, "18")).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Failed(_))).await;
    assert_eq!(it.status, Status::Failed("[youtube] abc: Video unavailable".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_video_stops_ytdlp_at_once() {
    let (m, dl, _) = setup("stop", "slow");
    let id = m.add(video(&dl, "18")).unwrap();
    let it = wait_for(&m, id, |_| m.get(id).is_some_and(|i| i.downloaded > 0)).await;
    assert_eq!(it.path, dl.join("Clip [abc] 360p.mp4"), "named as soon as yt-dlp said where it was going");
    assert_eq!(it.size, Some(650_000), "the size yt-dlp announced before starting");
    let asked = Instant::now();
    m.pause(id);
    wait_for(&m, id, |s| *s == Status::Paused).await;
    assert!(asked.elapsed() < Duration::from_secs(2), "took {:?}", asked.elapsed());
    let frozen = m.get(id).unwrap().downloaded;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(m.get(id).unwrap().downloaded, frozen, "nothing is still writing progress");
}

#[tokio::test(flavor = "multi_thread")]
async fn one_page_in_two_formats_is_two_downloads() {
    let (m, dl, _) = setup("dupes", "slow");
    let a = m.add_detailed(video(&dl, "18")).unwrap();
    let b = m.add_detailed(video(&dl, "140")).unwrap();
    let again = m.add_detailed(video(&dl, "18")).unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!((again.id, again.outcome), (a.id, Outcome::Existing));
    assert!(m.add(video(&dl, "18 --exec rm")).is_err(), "only a format selector is accepted");
    m.remove(a.id, true);
    m.remove(b.id, true);
}
