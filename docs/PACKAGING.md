# Packaging and signing

How the release artifacts are produced, and what is still needed to sign them. The commands to
build each one are in [BUILDING.md](BUILDING.md#packaging).

## What a release contains

| Artifact | Built by | Contents |
|---|---|---|
| `reIDM-setup-<v>.exe` | Inno Setup, `packaging/windows/reidm.iss` | `reidm.exe`, `reidm-gui.exe`, `reidm-host.exe`, the browser extension, docs, icon; registers both native-messaging hosts (`com.zektopic.reidm` for Chrome, Edge, Brave and Vivaldi; `com.tonec.idm` for Chrome, Edge, Brave and Firefox) and a login autostart; Start-menu entries for the user guide and the extension folder |
| `reidm_<v>-1_amd64.deb` | `cargo deb -p reidm` | the three binaries, the browser extension in `/usr/share/reidm/extension`, a `.desktop` entry, hicolor icons, a systemd user unit, both native-messaging hosts system-wide for Chrome, Chromium and Edge (`com.tonec.idm` also for Firefox), and the docs |
| `reIDM-<v>-universal.dmg` | `packaging/macos/make-dmg.sh` | a drag-to-Applications disk image holding `reIDM.app` (universal, with icon): the window, the service and the native host in `Contents/MacOS`, the extension in `Contents/Resources` |
| `reidm-extension-<v>.zip` | `zip` in the release workflow | the browser extension alone |

**The macOS disk image** opens to a window showing `reIDM.app`, an arrow, and a link to
`/Applications` over a drawn background — drag one onto the other and that's the installation.
There is no install script: the service and the native host sit beside the window's binary in
`Contents/MacOS`, so the first time reIDM opens, the service registers the native host with the
browsers and copies the extension out, as it does at every start.

- The window's layout (`.DS_Store`) is written by [dmgbuild](https://github.com/dmgbuild/dmgbuild)
  from `packaging/macos/dmg-settings.py`, with no Finder scripting, so it builds on a CI runner
  with nobody logged in. `python3 -m pip install dmgbuild` (or `pipx install dmgbuild`) before
  running `make-dmg.sh`.
- The background is `packaging/macos/background.png` and `background@2x.png`, drawn by
  `tools/make_dmg_background.py` (Pillow, Open Sans) and joined into one Retina-aware TIFF with
  `tiffutil` at build time. The icon positions in `dmg-settings.py` and in the drawing script
  must agree.
- For the command line, link the binaries inside the app onto your `PATH`:

  ```sh
  sudo ln -sf /Applications/reIDM.app/Contents/MacOS/reidm /usr/local/bin/reidm
  ```

- To start reIDM at login rather than on demand, add it in *System Settings › General › Login
  Items* (the window opens; closing it leaves the service running). It isn't needed for the
  browser: the extension starts reIDM when it isn't running.

**What the packages don't contain: yt-dlp, Deno, ffmpeg.** reIDM fetches them at run time,
into `tools/` in the user's data folder, the first time a video download needs them, and checks
each against the SHA-256 its release publishes (see
[BROWSER_INTEGRATION.md](BROWSER_INTEGRATION.md#youtube)). Bundling them would add about 150 MB to
every package, and a bundled yt-dlp stops working with YouTube within weeks. The `.deb`
*recommends* `ffmpeg`, so apt installs it by default.

**The browser extension in every package.** The service looks for the extension next to its own
binary — `extension/` beside `reidm.exe`, `../share/reidm/extension` from `/usr/bin` or
`~/.local/bin`, `Resources/extension` in the app bundle — and copies it to
`browser-extension/` in the user's data folder at every start. Browsers load it from there, so
updating reIDM updates the extension without the user re-loading it. Its ID,
`pgeonmdaagbiccimgojpafdjcipcfnlc`, is fixed by the public `key` in `extension/manifest.json`,
which is what lets the `com.zektopic.reidm` host manifests name it in `allowed_origins` ahead of
time.

## macOS signing

Everything is **ad-hoc signed** by default. That is not cosmetic: `lipo` does not carry code
signatures over to the universal binary it produces, and the macOS kernel refuses to execute an
unsigned arm64 binary. The symptom on Apple silicon is a bare `Killed: 9` with no dialog, so
`make-dmg.sh` re-signs each binary after `lipo` and then signs the bundle.

Ad-hoc signing is **not enough to distribute**. On another Mac, Gatekeeper will refuse the app with
"damaged and can't be opened" because the download carries a quarantine flag and there is no
Developer ID signature to validate. Two things fix that, and both need an Apple Developer account:

1. **A Developer ID Application certificate** (`.p12` plus its password).
2. **Notarization**, which needs an Apple ID, an app-specific password, and the Team ID — or an
   App Store Connect API key.

The workflow already implements both. Add these repository secrets and the next tagged release is
signed and notarized with no code change:

| Secret | What it is |
|---|---|
| `MACOS_CERT_P12` | the Developer ID Application certificate, base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `MACOS_CERT_PASSWORD` | the password you set when exporting the `.p12` |
| `MACOS_SIGN_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)`; `security find-identity -v -p codesigning` lists it |
| `MACOS_APPLE_ID` | the Apple ID used for notarization |
| `MACOS_APPLE_PASSWORD` | an [app-specific password](https://support.apple.com/en-us/102654) for that Apple ID |
| `MACOS_TEAM_ID` | the 10-character team identifier |

With `MACOS_CERT_P12` and `MACOS_APPLE_ID` present, `make-dmg.sh` signs with the real identity
(hardened runtime, timestamped) and runs `notarytool submit --wait` followed by `stapler staple`.
With either missing it prints a note and ships an ad-hoc signed build.

**Not implemented:** a hardened runtime needs entitlements for anything unusual. reIDM only needs
network client access, which is allowed by default, so no entitlements file is shipped yet. If
notarization ever rejects the build for a missing entitlement, add one next to `make-dmg.sh` and
pass it with `--entitlements`.

## Windows and Linux signing

Neither is signed.

* **Windows**: SmartScreen shows "Windows protected your PC" for an unknown publisher. Removing that
  needs an EV or OV code-signing certificate and `signtool`. Inno Setup can sign both the installer
  and the packaged binaries (`SignTool=` in the `.iss`).
* **Linux**: a `.deb` is not signed; repository metadata signing (`aptly`/`reprepro`) is what users
  actually verify, and there is no apt repository here yet.

## Verifying what you shipped

```sh
# macOS: check the signature and what a notarized build should report
codesign --verify --verbose=2 reIDM.app
spctl --assess --type execute --verbose reIDM.app     # "accepted" only when notarized
xcrun stapler validate reIDM-<v>-universal.dmg

# Debian: confirm the binaries and the native-messaging manifests are inside
dpkg -c reidm_<v>-1_amd64.deb | grep -E 'usr/bin|native-messaging|applications|icons'
```
