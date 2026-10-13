#!/usr/bin/env python3
"""Exercise reIDM over its WebSocket exactly the way the official IDM extension does.

This is the integration test for the browser hand-off. It reproduces the exchange
`background.js` performs when the browser is about to save a file:

  1. connect with the extension's sub-protocol and Origin
  2. hello (type 2)            -> expect the ack (type 3) and settings (type 4)
  3. a type-13 frame with flag 128 set, meaning "the browser is saving this file"
                               -> expect a type-1 reply whose result is 3, "taken",
                                  which is what makes the extension cancel the
                                  browser's own download
  4. keep-alive (type 0) while it runs, as the extension does every 15 seconds
  5. the file must arrive byte-for-byte

Usage:
    python3 tools/extension-flow-test.py \
        --ws 61001 \
        --api http://127.0.0.1:61002 \
        --token "$(cat ~/.local/share/reidm/api-token)" \
        --url http://127.0.0.1:8000/payload.bin \
        --sha256 <sha256 of payload.bin>

Start a daemon first (`reidm daemon`), and serve the payload from a local HTTP
server that supports Range requests.
"""
import argparse
import asyncio
import hashlib
import json
import sys
import urllib.request

try:
    import websockets
except ImportError:
    sys.exit("this script needs the 'websockets' package: pip install websockets")

SUBPROTOCOL = "plugin.v3.internetdownloadmanager.com"
# The exact hello frame shape the extension sends (see docs/PROTOCOL.md).
HELLO = "MSG#1#2#0#5:113:93:1031:0:16845059:0:15:3,112=8:Chrome/1,113=3:abc;"
EXT_ORIGIN = "chrome-extension://ngpampappnmepgilojfohadhhmbhlaek"

T_KEEPALIVE, T_REPLY, T_HELLO, T_INTERCEPT = 0, 1, 2, 13
T_HELLO_ACK, T_SETTINGS = 3, 4
R_TAKEN = 3
K_URL, K_NAME = 6, 100


def frame(msg_id, ty, flags, args, keys):
    """Serialise one frame: MSG#id#type#class#flags[:args][,k=v|k=len:bytes];"""
    s = f"MSG#{msg_id}#{ty}#0#{flags}"
    if args:
        s += ":" + ":".join(str(a) for a in args)
    for k, v in keys.items():
        if isinstance(v, int):
            s += f",{k}={v}"
        else:
            s += f",{k}={len(v.encode())}:{v}"
    return s + ";"


def parse(text):
    """(type, args) from a frame."""
    head, _, rest = text.partition(":")
    args = [int(a) for a in rest.rstrip(";").split(":") if a.strip().lstrip("-").isdigit()]
    return int(head.split("#")[2]), args


class Daemon:
    def __init__(self, api, token):
        self.api, self.token = api, token

    def downloads(self):
        r = urllib.request.Request(self.api + "/api/downloads", headers={"X-Reidm-Token": self.token})
        with urllib.request.urlopen(r, timeout=10) as f:
            return json.loads(f.read())

    def item(self, did):
        return next(i for i in self.downloads() if i["id"] == did)


async def run(a):
    d = Daemon(a.api, a.token)
    async with websockets.connect(
        f"ws://127.0.0.1:{a.ws}/?cid=1&rnd=2",
        subprotocols=[SUBPROTOCOL],
        additional_headers={"Origin": EXT_ORIGIN},
    ) as ws:
        # 1. hello
        await ws.send(HELLO.encode())
        got = []
        for _ in range(2):
            text = (await asyncio.wait_for(ws.recv(), timeout=5)).decode()
            got.append(parse(text)[0])
            print(f"  <- type {got[-1]}: {text[:96]}")
        assert got == [T_HELLO_ACK, T_SETTINGS], f"expected ack then settings, got {got}"

        # 2. the browser is saving a file
        before = {i["id"] for i in d.downloads()}
        cap = frame(7, T_INTERCEPT, 128 | 1, [0], {K_URL: a.url, K_NAME: a.name})
        print(f"  -> {cap[:110]}")
        await ws.send(cap.encode())
        ty, args = parse((await asyncio.wait_for(ws.recv(), timeout=5)).decode())
        assert ty == T_REPLY, f"expected a type-1 reply, got type {ty}"
        assert args == [7, R_TAKEN], f"expected [request 7, taken({R_TAKEN})], got {args}"
        print(f"  <- the extension is told to cancel its own download (result {R_TAKEN})")

        # 3. it must actually be queued
        new = [i for i in d.downloads() if i["id"] not in before]
        assert len(new) == 1, f"expected exactly one new download, got {len(new)}"
        did = new[0]["id"]
        print(f"  queued as #{did}")

        # 4. keep-alive until it finishes
        state, item = "Queued", new[0]
        for _ in range(a.timeout * 2):
            await asyncio.sleep(0.5)
            item = d.item(did)
            state = item["status"]["state"]
            if state in ("Done", "Failed"):
                break
            await ws.send(frame(8, T_KEEPALIVE, 1, [], {}).encode())
            try:
                await asyncio.wait_for(ws.recv(), timeout=2)
            except asyncio.TimeoutError:
                pass
        print(f"  final state: {state} ({item['downloaded']}/{item.get('size')} bytes)")
        assert state == "Done", f"download did not complete: {item['status']}"

    with open(item["path"], "rb") as f:
        got = hashlib.sha256(f.read()).hexdigest()
    assert got == a.sha256, f"hash mismatch\n  got      {got}\n  expected {a.sha256}"
    print(f"  OK  sha256 {got}")
    print("\nPASS: the extension capture flow works end to end.")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--ws", type=int, default=61001, help="daemon WebSocket port (default 61001)")
    p.add_argument("--api", default="http://127.0.0.1:61002", help="daemon HTTP API base")
    p.add_argument("--token", required=True, help="contents of the daemon's api-token file")
    p.add_argument("--url", required=True, help="URL the extension would hand over")
    p.add_argument("--sha256", required=True, help="expected SHA-256 of the downloaded file")
    p.add_argument("--name", default="captured-by-extension.bin", help="filename the extension suggests")
    p.add_argument("--timeout", type=int, default=120, help="seconds to wait for the download")
    a = p.parse_args()
    try:
        asyncio.run(run(a))
    except AssertionError as e:
        sys.exit(f"FAIL: {e}")


if __name__ == "__main__":
    main()
