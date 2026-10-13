#!/usr/bin/env python3
"""Write docs/installer-manifest.tsv: one row per file recovered from the IDM installer overlay."""
import hashlib, os, re, subprocess, sys
raw = sys.argv[1]; out = sys.argv[2]
rows = ["index\toverlay_offset\tbytes\tsha256\ttype\tpdb_path"]
for f in sorted(os.listdir(raw)):
    p = os.path.join(raw, f); d = open(p, 'rb').read()
    idx, off = re.match(r'(\d+)_([0-9a-f]+)\.bin', f).groups()
    ty = subprocess.run(['file', '-b', p], capture_output=True, text=True).stdout.strip()[:70]
    m = re.search(rb'[A-Za-z]:\\[\w\\. -]+?\.pdb', d)
    rows.append(f"{idx}\t0x{int(off,16):x}\t{len(d)}\t{hashlib.sha256(d).hexdigest()}\t{ty}\t{m.group(0).decode() if m else ''}")
open(out, 'w').write("\n".join(rows) + "\n")
print(len(rows) - 1, "files")
