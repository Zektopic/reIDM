#!/usr/bin/env python3
"""Load reIDM's MV3 extension in a real Chromium and prove a download is captured.

This verifies what no unit test can: that Chrome itself starts the service worker, and
that a download Chrome begins ends up in reIDM instead of the browser's download folder.
It is the regression test for two bugs an earlier version had — a top-level `await`
before the listeners were registered (Chrome then never starts the worker, silently),
and cancelling the browser's download before reIDM had agreed to take it (which loses
the file outright if reIDM is not running).

Needs playwright with its bundled Chromium, and a display (use `xvfb-run` on a headless
machine, or `DISPLAY=:99` with an Xvfb). Start a daemon first.

    pip install playwright && playwright install chromium
    reidm daemon &
    python3 tools/browser-extension-test.py --extension extension \
        --daemon http://127.0.0.1:61002 --token-file ~/.local/share/reidm/api-token \
        --page http://127.0.0.1:8000/ --file http://127.0.0.1:8000/files/test.zip

Exits non-zero unless reIDM ended up with the file *and* the browser kept nothing.
"""
import hashlib, json, os, shutil, sys, time, urllib.request
from playwright.sync_api import sync_playwright

EXT = os.environ.get("REIDM_EXT", "extension")
CHROME = os.environ.get("CHROME_PATH") or None  # None = playwright's own
PROFILE = os.environ.get("REIDM_EXT_PROFILE", "/tmp/reidm-ext-profile")
DL_DIR = os.environ.get("REIDM_EXT_DL", "/tmp/reidm-ext-downloads")
API = os.environ.get("REIDM_API", "http://127.0.0.1:61002")
TOKEN = open(os.environ.get("REIDM_TOKEN_FILE", os.path.expanduser("~/.local/share/reidm/api-token"))).read().strip()
PAGE = os.environ.get("REIDM_PAGE", "http://127.0.0.1:8000/")
import time as _t
BASE_FILE = os.environ.get("REIDM_FILE", "http://127.0.0.1:8000/files/test.zip")
# A fresh URL each run, so reIDM's "already downloaded" short-circuit does not mask a failure.
FILE = f"{BASE_FILE}?run={int(_t.time())}"

def api(path):
    r = urllib.request.Request(API + path, headers={"X-Reidm-Token": TOKEN})
    with urllib.request.urlopen(r, timeout=10) as f:
        return json.loads(f.read())

def main():
    shutil.rmtree(PROFILE, ignore_errors=True)
    shutil.rmtree(DL_DIR, ignore_errors=True)
    os.makedirs(DL_DIR, exist_ok=True)
    before = {i["id"] for i in api("/api/downloads")}

    with sync_playwright() as p:
        ctx = p.chromium.launch_persistent_context(
            PROFILE, executable_path=CHROME, headless=False,
            args=[f"--disable-extensions-except={EXT}", f"--load-extension={EXT}",
                  # Chrome 137+ silently ignores --load-extension without this.
                  "--disable-features=DisableLoadExtensionCommandLineSwitch",
                  "--no-first-run", "--no-default-browser-check"],
            accept_downloads=True, downloads_path=DL_DIR)
        try:
            page = ctx.new_page()
            page.goto(PAGE, wait_until="domcontentloaded")
            time.sleep(2)
            # Is the service worker actually running? (This is what a top-level await breaks.)
            workers = [w.url for w in ctx.service_workers]
            print(f"service workers: {len(workers)}")
            for w in workers:
                print("  ", w)

            print("navigating to the file to trigger a browser download…")
            try:
                page.goto(FILE, wait_until="domcontentloaded", timeout=15000)
            except Exception as e:
                print(f"  (navigation ended: {type(e).__name__})")
            time.sleep(6)

            after = {i["id"] for i in api("/api/downloads")}
            new = sorted(after - before)
            print(f"new downloads in reIDM: {new}")
            for n in new:
                it = next(i for i in api("/api/downloads") if i["id"] == n)
                print(f"  #{n} {it['status']['state']} {it['downloaded']}/{it.get('size')} {it['request']['url']}")
            captured = any(FILE in next(i for i in api('/api/downloads') if i['id'] == n)['request']['url'] for n in new)
        finally:
            ctx.close()

    # Only now is Chrome's transient UUID temp file gone; checking earlier is a race.
    time.sleep(1)
    left_behind = [f for f in os.listdir(DL_DIR) if os.path.isfile(os.path.join(DL_DIR, f))]
    print(f"files kept by the browser: {left_behind}")
    ok = captured and not left_behind
    print("\nRESULT:", "PASS — reIDM captured it, browser kept nothing" if ok else "FAIL")
    return 0 if ok else 1

sys.exit(main())
