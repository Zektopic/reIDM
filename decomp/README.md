# decomp/

Raw Ghidra 12.1.4 headless decompiler output (`tools/decomp.sh`). Function names are Ghidra's defaults (`FUN_<addr>`); nothing is renamed or cleaned.
Derived from proprietary software — **keep private**, see `../NOTICE.md`.

| File | Source binary (installer index) | SHA-256 of source | Why |
|---|---|---|---|
| IDMMsgHost.exe.c | 126 | dba25a49adb88f675db26d2dc7a0fa9d4a5db2326858cb9d2515f6f34b8e0b65 | native-messaging relay (read this one first) |
| IDMNetMon.exe.c | 072 (x86 DLL, internally "idman.exe") | see `../docs/installer-manifest.tsv` | WebSocket server on :1001 |
| idmnmcl.dll.c | 180 | see manifest | named-pipe client used by the integrations |
| IDMan.exe.c | 005 | see manifest | main application (unanalysed beyond bulk decompile) |

The remaining 188 recovered files (drivers, shell extensions, OpenSSL, language files…) were not decompiled; their hashes are in the manifest.
