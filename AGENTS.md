# Working on reIDM — a guide for agents and new contributors

reIDM is a cross-platform download manager in Rust: a background **service** (`reidm daemon`)
owns every download; a native **window** (`reidm-gui`, egui) and a **CLI** talk to it over a
local HTTP API; a Chromium **extension** hands it the browser's downloads. It also speaks the
original IDM extension's protocol. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before
changing anything structural, and **[docs/DEVLOG.md](docs/DEVLOG.md) for why things are the way
they are** — add an entry there for every change that involves a decision.

## Layout

| Path | What |
|---|---|
| `crates/reidm-core` | the engine: segmented HTTP downloads (`lib.rs`), videos through yt-dlp (`media.rs`), torrents through librqbit (`torrent.rs`), categories (`category.rs`), the limiter, scheduler, lists, site grabber, update check |
| `crates/reidm` | the `reidm` binary: CLI, daemon, HTTP API (`web.rs`), IDM WebSocket protocol (`ws.rs`, `session.rs`), native-host registration (`install.rs`) |
| `crates/reidm-host` | the native-messaging host the browser starts |
| `crates/reidm-gui` | the desktop window (`app.rs`), its API client (`api.rs`), playlist and torrent windows |
| `crates/reidm-proto` | the IDM protocol's binary messages |
| `extension/` | reIDM's own MV3 extension |
| `docs/` | user and developer documentation — keep it true when behaviour changes |
| `packaging/`, `.github/workflows/` | installers and CI. Linux CI and the `.deb` build run on the self-hosted runner `manupa-hnx`; Windows and macOS on GitHub's |
| `decomp/`, `samples/` | reverse-engineering material; `samples/` is untrusted vendor data (read-only, `python3 -I`) |

## Before you call something done

```sh
cargo test --workspace --locked                                   # ~105 tests, all offline
cargo clippy --workspace --all-targets --locked -- -D warnings    # also with --target x86_64-pc-windows-gnu
xvfb-run -a node tools/capture-test.mjs                           # the real-browser suite (50 checks)
REIDM_PROMPT=1 xvfb-run -a node tools/capture-test.mjs            # with "Ask where to save" on
REIDM_ONLY_GRABBER=1 xvfb-run -a node tools/capture-test.mjs      # the resource grabber
```

For changes to the engine's transfer path, its saving or its memory use, also measure on imitated
machines (Linux; cgroup limits via `systemd-run --user`; a throw-away service on its own ports):

```sh
cargo build --release --locked -p reidm
node tools/profile-test.mjs                    # potato (½ core, 256 MB), laptop (2 cores, 1 GB), desktop
```

It downloads from a local server and compares every file byte for byte, kills one download with
SIGKILL part way and checks that it resumes intact, and measures speed, CPU per GiB, memory and
idle cost with a long list. Record the table in the devlog when the numbers move.

The service also has a load test for responsiveness: a large download polled like the window and
restarted, repeated playlist-sized batch adds and purges, and a server that never answers (Linux;
ports 61201/61202 and a local site on 61290):

```sh
node tools/stress-test.mjs                     # ~2 GiB download, ten 300-link cycles
node tools/stress-test.mjs --profile potato    # the same under half a core and 256 MB
```

It fails if a row poll takes over 2 s, Pause on a silent server over 3 s, a 300-item purge over
30 s, or memory grows by more than 150 MB over the cycles. `--profile laptop` caps it at two
cores and 1 GB; the site stays unlimited, so the cap is on reIDM, as on a real machine.

The API and CLI have a live permutation test: checksums (right and wrong), an HTTP proxy end to
end, queue order and moving to the top, the free-space pause and carry-on, restart, remove with and
without files, and `reidm list`/`add --later`/`rm` (Linux; ports 61401/61402, a site on 61490, a
proxy on 61491):

```sh
node tools/api-test.mjs
```

CI installs the **newest stable** Rust, which may be newer than yours, and a new deprecation then
fails `-D warnings` there first. Before pushing, lint with CI's version as well (it is printed in
the *Install Rust* step), in its own target folder so your normal build isn't redone:

```sh
rustup toolchain install 1.99.1 -c clippy -t x86_64-pc-windows-gnu
CARGO_TARGET_DIR=target/ci cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings
```

The browser suite needs a Chromium (`npx playwright install chromium`; it finds the newest) and
ports 61001/61002 free — stop any running reIDM first.

The extension also builds for **Firefox 128+** from the same sources (a shim and a Firefox
manifest; see `extension/README.md`):

```sh
node tools/build-firefox-extension.mjs --self-test
npx web-ext lint --source-dir dist/firefox-extension
xvfb-run -a npx web-ext run --source-dir dist/firefox-extension --firefox=/usr/bin/firefox --no-reload
```

`web-ext run` installs it as a temporary add-on; its background pings the running service, so a
throw-away endpoint on 61002 shows that it started. Firefox has no MV3 service workers or
`onDeterminingFilename`: the build handles the first, and the worker's non-Chromium path pauses
downloads on creation for the second.
 For macOS lint without a Mac:
`CC_aarch64_apple_darwin=true AR_aarch64_apple_darwin=true cargo clippy … --target aarch64-apple-darwin`.

Video and torrent features have offline tests (a stand-in yt-dlp shell script in
`tests/media.rs`; a local librqbit seeder in `tests/torrent.rs`); real-world checks against
YouTube and the Debian swarm are done by hand and recorded in the devlog.

## House rules

- **Never lose a user's download.** Resume rather than restart; refuse rather than splice two
  different files; keep partial data when in doubt. Every engine change needs a test that
  proves this.
- **The browser must never end up with nobody downloading the file.** If reIDM can't take a
  download, the browser keeps it.
- **Simple by default.** New options go behind *Advanced* unless most users need them; defaults
  must be right for most people.
- **Explain, don't just fail.** Error messages say what happened and what to do.
- Comments explain *why*, in full sentences; match the surrounding style. No dead code.
- Docs, CHANGELOG and DEVLOG change in the same commit as the behaviour.
- Commits end with the `Co-Authored-By` trailer the repository uses; stage paths explicitly
  (an untracked `archive-IQ0Adc/` in some checkouts is not ours).

## Traps already met (see the devlog for each)

- `Path::canonicalize` on Windows returns `\\?\C:\…`, which browsers can't launch — strip it
  for anything handed to another program.
- Processes started from an elevated session (SSH on Windows is elevated) can't be stopped by the
  user's normal-rights installer.
- YouTube needs yt-dlp **with a JavaScript runtime** (Deno); one connection is throttled and cut
  off, so yt-dlp runs with `formats=dashy` and parallel fragments.
- librqbit pauses restored torrents asynchronously; a pause can land during the start-up check.
- Windows notifications may be switched off globally — never rely on them alone.
- CI's newer Rust deprecated `Atomic*::fetch_update` (renamed `try_update`); neither name builds
  warning-free on both old and new toolchains, so use a `compare_exchange_weak` loop.
