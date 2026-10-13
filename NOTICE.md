# Notice

reIDM is an independent project and is **not affiliated with, endorsed by, or derived from source code of Tonec Inc. / Tonec FZE**.
"Internet Download Manager" and "IDM" are trademarks of their owner.

## What `decomp/` is

`decomp/` contains machine-generated Ghidra decompiler output for four binaries recovered from the vendor installer
`idman643build15.exe` (SHA-256 `a902e771…bbfd9`), plus `docs/installer-manifest.tsv` (hashes of every recovered file).
It exists to document how the browser-integration protocol works so that reIDM can interoperate with the official extension.

* It is derived from proprietary software. The original binaries are **not** in this repository (see `.gitignore`).
* Keep this repository **private**. Do not publish or redistribute `decomp/` without checking the vendor's licence terms
  and the law where you live; interoperability exceptions to decompilation restrictions differ by jurisdiction.
* No reIDM source was copied from the decompiled output: the protocol was re-implemented from observed message formats
  (extension JavaScript, `docs/PROTOCOL.md`). The decompiler output is reference material only.

## What reIDM deliberately does not do

* It does not implement, bypass, emulate or reset IDM licensing, registration or trial logic.
* It does not bundle the vendor's installer, binaries or browser extension.
