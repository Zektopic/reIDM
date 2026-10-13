# Packaging and signing

How the three release artifacts are produced, and what is still needed to sign them.

## What a release contains

| Artifact | Built by | Contents |
|---|---|---|
| `reIDM-setup-<v>.exe` | Inno Setup, `packaging/windows/reidm.iss` | `reidm.exe`, `reidm-gui.exe`, `reidm-host.exe`, docs, icon; registers the `com.tonec.idm` native-messaging host and a login autostart |
| `reidm_<v>-1_amd64.deb` | `cargo deb -p reidm` | the three binaries, a `.desktop` entry, hicolor icons, a systemd user unit, and native-messaging manifests for Chrome/Chromium/Edge/Firefox |
| `reIDM-<v>-universal.dmg` | `packaging/macos/make-dmg.sh` | a `reIDM.app` bundle (universal, with icon), the two CLI binaries, `Install reIDM.command`, docs |

The DMG is **not** a bare copy of binaries: `reIDM.app` is assembled with an `Info.plist` and an
`.icns`, so it can be dragged to `/Applications` and launched from Finder.

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
