# Building and testing

## Prerequisites

- **Rust**, stable (`rustup` recommended). The workspace uses the 2021 edition and builds with
  the current stable toolchain, which is what CI uses.
- **Linux:** the windowing and GL headers the desktop app links against:

  ```sh
  sudo apt install libxkbcommon-dev libwayland-dev libxcb1-dev libx11-dev libgl1-mesa-dev libfontconfig1-dev pkg-config
  ```

- **macOS:** Xcode's command-line tools (`xcode-select --install`).
- **Windows:** the MSVC toolchain (Visual Studio Build Tools with *Desktop development with C++*).

## Build

```sh
git clone https://github.com/Zektopic/reIDM && cd reIDM
cargo build --release
```

| Binary | |
|---|---|
| `target/release/reidm` | the service and command line |
| `target/release/reidm-gui` | the desktop window |
| `target/release/reidm-host` | the native-messaging helper; must sit next to `reidm` |

To try it from the build tree: start `target/release/reidm-gui` (it starts the service), then
load `extension/` in the browser as described in
[BROWSER_INTEGRATION.md](BROWSER_INTEGRATION.md). The service registers the native-messaging
helper from wherever `reidm-host` is, so the extension can start the build you're running.

## Tests

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

CI runs both on Linux, Windows and macOS for every push and pull request.

| Suite | Where | What |
|---|---|---|
| unit tests | `src/` of each crate | the IDM wire format, segment claiming and splitting, the limiter, schedules, `.ef2`, batch patterns, URL keys, file names, host manifests, rate parsing |
| `resume` | `crates/reidm-core/tests/resume.rs` | against a local HTTP server: resuming with and without range support, after the part file is lost, zero-byte files, duplicate names and addresses, removal and purge, the concurrency limit, batch adds, the site grabber |
| `engine` | `crates/reidm-core/tests/engine.rs` | a slow connection's range taken over, servers that refuse extra connections, the global and per-download limits, a file that changes on the server (with and without an `ETag`), refreshing an address, the scheduled queue, checked adds and web pages posing as files |

### The real-browser tests

Unit tests can't show that Chrome hands a download to the extension, or that the browser keeps
nothing afterwards. `tools/capture-test.mjs` does: it drives a real Chromium over the DevTools
protocol, against a local test site and a real reIDM service in a throw-away data folder, using
trusted mouse events (the extension ignores synthetic clicks).

```sh
cargo build -p reidm -p reidm-host
xvfb-run -a node tools/capture-test.mjs --chrome /path/to/chrome     # or with a display: node tools/…
```

It needs Node 22 or newer (for its built-in `WebSocket`) and nothing else — no npm packages. Any Chromium works; Chrome for
Testing or Playwright's build (`npx playwright install chromium`) is the easy way to get one
whose path is stable. It checks, among other things, that:

- a file link is taken on click and the browser never requests it;
- a download only recognisable from its response headers is taken before the browser saves
  anything — even a 2 KB file, and with *Ask where to save each file* on (`REIDM_PROMPT=1`);
- cookies reach reIDM; form POSTs, Alt+click and links that turn out to be web pages stay with
  the browser;
- with the service stopped, the extension starts it through the native host and still takes the
  download — and without the host, the browser simply downloads it;
- the resource grabber finds links, images, media and script-built addresses, and sends them.

| Option | |
|---|---|
| `--chrome PATH` or `CHROME_PATH` | the browser to drive |
| `--bin DIR` | where the reIDM binaries are (default `target/debug`) |
| `--extension DIR` | the extension to load (default `extension/`) |
| `--site-port`, `--cdp-port` | ports for the test site (18871) and DevTools (9333) |
| `REIDM_PROMPT=1` | turn on the browser's *Ask where to save each file* |
| `REIDM_ONLY_GRABBER=1` | run only the resource-grabber checks |
| `REIDM_SHOT_CMD`, `REIDM_SHOT_DIR` | a command that takes a screenshot, run at key moments, and where to keep them |
| `KEEP=1` | keep the temporary folder (profile, logs, downloads) for inspection |

It exits non-zero if any check fails. The service it starts uses the usual ports, so stop any
running reIDM first.

Two older scripts remain for the IDM-extension path: `tools/extension-flow-test.py` speaks the
IDM WebSocket protocol exactly as Tonec's extension does, and `tools/browser-extension-test.py`
is an earlier Playwright smoke test of reIDM's extension.

## Cross-compiling for Windows from Linux

Useful for checking that Windows-only code compiles:

```sh
sudo apt install gcc-mingw-w64-x86-64
rustup target add x86_64-pc-windows-gnu
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo build --release --target x86_64-pc-windows-gnu
```

Release builds for Windows are made with MSVC on Windows (below).

## Packaging

Each installer is built by one command; [PACKAGING.md](PACKAGING.md) covers what's inside them
and code signing.

| | Command | Needs |
|---|---|---|
| Windows `reIDM-setup-<v>.exe` | `iscc /DAppVersion=<v> packaging\windows\reidm.iss` | [Inno Setup 6](https://jrsoftware.org/isinfo.php); a release build |
| macOS `reIDM-<v>-universal.dmg` | `packaging/macos/make-dmg.sh <v> reidm reidm-host reidm-gui out.dmg` | universal binaries (`lipo` of the x86_64 and arm64 builds) |
| Debian `reidm_<v>-1_amd64.deb` | `cargo deb -p reidm --no-build` | `cargo install cargo-deb`; a release build |
| `reidm-extension-<v>.zip` | `cd extension && zip -r -X ../reidm-extension-<v>.zip .` | |

The icon set — `packaging/icon/*` and `extension/icons/*` — is generated, not drawn by hand:

```sh
python3 -I tools/make_icon.py
```

It uses only the Python standard library.

## Releases

Pushing a tag `v<version>` runs `.github/workflows/release.yml`: it tests and builds on all three
systems, makes the four artifacts above, and publishes a GitHub release with them. Before
tagging:

1. set the version in the workspace `Cargo.toml` and run `cargo update --workspace` so
   `Cargo.lock` follows;
2. bump `version` in `extension/manifest.json` if the extension changed;
3. add the release to `CHANGELOG.md`.

## The decompiled reference

`decomp/` holds Ghidra output for IDM's binaries, used to match its protocol and formats
([REVERSE_ENGINEERING.md](REVERSE_ENGINEERING.md), [NOTICE.md](../NOTICE.md)). Nothing in it is
built or linked. `tools/decomp.sh` regenerates a file from a binary with headless Ghidra;
`tools/unpack_idm.py` and `tools/make_manifest.py` take an installer apart for analysis.
