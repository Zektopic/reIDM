# Notice

reIDM is an independent project and is **not affiliated with, endorsed by, or derived from source code of Tonec Inc. / Tonec FZE**.
"Internet Download Manager" and "IDM" are trademarks of their owner.

## What `decomp/` is

`decomp/` contains machine-generated Ghidra decompiler output for four binaries recovered from the vendor installer
`idman643build15.exe` (SHA-256 `a902e771…bbfd9`), plus `docs/installer-manifest.tsv` (hashes of every recovered file).
It exists to document how the browser-integration protocol works so that reIDM can interoperate with the official extension.

* It is derived from proprietary software. The original binaries are **not** in this repository (see `.gitignore`).
* No reIDM source was copied from the decompiled output: the protocol was re-implemented from observed message formats
  (extension JavaScript, `docs/PROTOCOL.md`). The decompiler output is reference material only.

### On publishing this

**This repository is public, by the owner's deliberate decision.** That is a knowing choice and not an oversight, but it
is worth stating plainly what it means:

* The decompiler output is derived from Tonec's proprietary binaries, and it is now readable by anyone. Whether that is
  lawful depends on where you are: the EU and the US recognise narrower interoperability rights than some other
  jurisdictions, and those exceptions generally attach to *decompiling for a purpose*, not to *republishing the result*.
  Publishing is the riskier half.
* Removing these files from the working tree in future would **not** unpublish them: they remain in the git history and
  stay downloadable from the existing commit URLs. Taking them down would mean rewriting history and force-pushing, or
  making the repository private again.
* The purpose here is interoperability with a piece of software the user already has, which is the use the exceptions
  are aimed at. Nothing in this repository implements or interferes with Tonec's licensing, registration or trial
  logic, and the vendor's binaries and installer are not redistributed.

## What reIDM deliberately does not do

* It does not implement, bypass, emulate or reset IDM licensing, registration or trial logic.
* It does not bundle the vendor's installer, binaries or browser extension.
