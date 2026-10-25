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
# Every run's arguments, one run per line.
echo "$*" >> "$here/runs.txt"
out=""; home=""; temp=""; prev=""; cookies=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  [ "$prev" = "--cookies" ] && cookies="$a"
  [ "$prev" = "-P" ] && case "$a" in home:*) home="${a#home:}" ;; temp:*) temp="${a#temp:}" ;; esac
  prev="$a"
done
[ -n "$home" ] && out="$home/$out"
printf '%s' "$temp" > "$here/temp.txt"
[ -n "$cookies" ] && { printf '%s' "$cookies" > "$here/cookies-at.txt"; ls -l "$cookies" | cut -c1-10 > "$here/cookies-mode.txt"; }
file=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/; s/%(title)\.150B/Clip/; s/%(id)s/abc/; s/%(height&{}p|audio)s/360p/')
case "$(cat "$here/mode" 2>/dev/null)" in
  oldpart) case " $* " in *" --concurrent-fragments "*) echo "ERROR: fragment 1 not found, unable to continue" >&2; exit 1 ;; esac ;;
  fail) echo "WARNING: [youtube] something minor" >&2; echo "ERROR: [youtube] abc: Video unavailable" >&2; exit 1 ;;
  slow) echo "REIDM-NAME $file"; echo "REIDM-SIZE 650000.0"
        i=0; while [ $i -lt 600 ]; do echo "REIDM-PROGRESS $((i * 1000)) 600000 NA NA 18"; i=$((i + 1)); sleep 0.05; done ;;
  # The video's addresses expire partway through the first run: the server answers 403.
  expire) if [ ! -e "$here/expired" ]; then
            touch "$here/expired"; echo "REIDM-NAME $file"
            echo "REIDM-PROGRESS 0 1000 NA NA 137"; echo "REIDM-PROGRESS 400 1000 NA NA 137"
            echo "ERROR: unable to download video data: HTTP Error 403: Forbidden" >&2; exit 1
          fi ;;
  forbidden) echo "ERROR: unable to download video data: HTTP Error 403: Forbidden" >&2; exit 1 ;;
  # A title in another encoding, on both outputs.
  latin1) printf 'REIDM-NAME %s\n' "$file"; printf 'caf\351 \377\376\n'; printf 'caf\351 on stderr\n' >&2 ;;
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
async fn deleting_a_running_video_stops_ytdlp_and_removes_its_pieces() {
    // A playlist item deleted while it downloads: the run must stop yt-dlp (the whole process
    // group), leave no pieces behind, and answer at once — the delete that used to hang.
    let (m, dl, tools) = setup("remove", "slow");
    let state = tools.parent().unwrap().to_path_buf();
    let id = m.add(video(&dl, "18")).unwrap();
    wait_for(&m, id, |_| m.get(id).is_some_and(|i| i.downloaded > 0)).await;
    let work = state.join("parts").join(format!("video-{id}"));
    assert!(work.exists(), "the work folder is there while it downloads");
    let asked = Instant::now();
    m.remove(id, true);
    let end = Instant::now() + Duration::from_secs(5);
    while work.exists() {
        assert!(Instant::now() < end, "the work folder outlived the delete by 5 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(asked.elapsed() < Duration::from_secs(5), "took {:?}", asked.elapsed());
    assert!(m.get(id).is_none(), "off the list");
    assert!(std::fs::read_dir(&dl).unwrap().next().is_none(), "nothing left in the download folder");
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

#[tokio::test(flavor = "multi_thread")]
async fn a_video_begun_over_one_connection_carries_on_that_way() {
    // reIDM 0.5.0 fetched videos over one connection; yt-dlp can't continue such a partial file
    // in pieces, and says "fragment 1 not found". The download must carry on, not fail.
    let (m, dl, tools) = setup("oldpart", "oldpart");
    let id = m.add(video(&dl, "18")).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Done | Status::Failed(_))).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    let args = std::fs::read_to_string(tools.join("args.txt")).unwrap();
    assert!(!args.lines().any(|a| a == "--concurrent-fragments"), "the second attempt is over one connection");
}

fn runs(tools: &Path) -> Vec<String> {
    std::fs::read_to_string(tools.join("runs.txt")).unwrap_or_default().lines().map(String::from).collect()
}

/// The value after `flag` in one run's arguments.
fn arg_of(run: &str, flag: &str) -> Option<String> {
    let mut it = run.split(' ');
    it.position(|a| a == flag)?;
    it.next().map(String::from)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_whose_addresses_expire_partway_carries_on_from_the_page() {
    // YouTube's addresses for a video's data expire after hours; a long download outlives them
    // and the server starts answering 403. The download must carry on with fresh ones — which
    // means reading the page again, never reusing what the last run was given.
    let (m, dl, tools) = setup("expire", "expire");
    let id = m.add(video(&dl, "137+bestaudio")).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Done | Status::Failed(_))).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    let runs = runs(&tools);
    assert_eq!(runs.len(), 2, "one more run after the 403: {runs:?}");
    for r in &runs {
        assert!(r.ends_with("-- https://www.youtube.com/watch?v=abc"), "each run starts from the page: {r}");
        assert!(r.contains("--continue"), "and continues the pieces it has: {r}");
    }
    assert!(runs[0].contains("--abort-on-unavailable-fragments"), "a missing piece stops yt-dlp rather than leaving a hole");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_refused_from_the_start_fails_at_once() {
    // Carrying on is for a download that was getting somewhere; one that never started fails
    // with yt-dlp's reason, without running it again and again.
    let (m, dl, tools) = setup("forbidden", "forbidden");
    let id = m.add(video(&dl, "18")).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Failed(_))).await;
    assert_eq!(it.status, Status::Failed("unable to download video data: HTTP Error 403: Forbidden".into()));
    assert_eq!(runs(&tools).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn output_in_another_encoding_does_not_stop_the_reading() {
    // A title in Latin-1 used to end the reading of yt-dlp's output; yt-dlp then blocked on a
    // full pipe and the download hung.
    let (m, dl, _) = setup("latin1", "latin1");
    let id = m.add(video(&dl, "18")).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Done | Status::Failed(_))).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cookies_file_is_private_and_goes_with_the_download() {
    let (m, dl, tools) = setup("cookies", "");
    let mut req = video(&dl, "18");
    req.cookie = Some("SID=secret".into());
    let id = m.add(req).unwrap();
    let it = wait_for(&m, id, |s| matches!(s, Status::Done | Status::Failed(_))).await;
    assert_eq!(it.status, Status::Done, "{:?}", it.status);
    assert_eq!(std::fs::read_to_string(tools.join("cookies-mode.txt")).unwrap().trim(), "-rw-------", "readable by this user only, from the start");
    let at = std::fs::read_to_string(tools.join("cookies-at.txt")).unwrap();
    assert!(at.contains(&format!("video-{id}")), "kept in the download's own folder: {at}");
    assert!(!Path::new(&at).exists(), "and gone when it is done");
}

#[tokio::test(flavor = "multi_thread")]
async fn ytdlp_gets_its_share_of_the_global_limit_and_a_new_limit_reaches_it() {
    let (m, dl, tools) = setup("limit", "slow");
    m.update_settings(reidm_core::SettingsPatch { speed_limit: Some(1_000_000), limit_enabled: Some(true), ..Default::default() }).unwrap();
    let a = m.add(video(&dl, "18")).unwrap();
    wait_for(&m, a, |_| m.get(a).is_some_and(|i| i.downloaded > 0)).await;
    assert_eq!(arg_of(&runs(&tools)[0], "--limit-rate").as_deref(), Some("1000000"), "alone: all of it");
    let b = m.add(video(&dl, "140")).unwrap();
    wait_for(&m, b, |_| runs(&tools).len() >= 2).await;
    assert_eq!(arg_of(&runs(&tools)[1], "--limit-rate").as_deref(), Some("500000"), "two running: half each");

    // A new global limit restarts yt-dlp with it (it continues its pieces), within seconds.
    m.update_settings(reidm_core::SettingsPatch { speed_limit: Some(200_000), ..Default::default() }).unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while runs(&tools).len() < 4 {
        assert!(Instant::now() < end, "not restarted: {:?}", runs(&tools));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for r in &runs(&tools)[2..] {
        assert_eq!(arg_of(r, "--limit-rate").as_deref(), Some("100000"), "{r}");
    }
    m.remove(a, true);
    m.remove(b, true);
}
