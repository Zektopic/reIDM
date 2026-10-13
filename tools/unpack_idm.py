#!/usr/bin/env python3
"""Scan the IDM installer overlay for zlib streams and extract them (analysis helper)."""
import sys, zlib, os, struct
src, out = sys.argv[1], sys.argv[2]
d = open(src, 'rb').read()
os.makedirs(out, exist_ok=True)
pos = 95232; i = 0
idx = []
while True:
    p = d.find(b'x\xda', pos)
    if p < 0: break
    try:
        o = zlib.decompressobj(); data = o.decompress(d[p:])
        used = len(d[p:]) - len(o.unused_data)
        if o.eof and len(data) > 0:
            # header before stream: look at 40 bytes before for metadata
            idx.append((p, used, len(data), d[max(0,p-24):p].hex()))
            open(f"{out}/{i:03d}_{p:08x}.bin", 'wb').write(data); i += 1
            pos = p + used; continue
    except Exception: pass
    pos = p + 1
for r in idx: print(r)
print(len(idx))
