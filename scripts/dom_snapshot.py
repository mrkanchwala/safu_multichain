"""Rendered-DOM snapshot of the built site, for "no visible change" refactors (B42, 2026-11-16).

    SAFU_POOL_NETWORK=mainnet npm run build
    SAFU_POOL_NETWORK=mainnet python3 scripts/dom_snapshot.py snapshot out.json   # preview needs the switch too
    python3 scripts/dom_snapshot.py diff before.json after.json

All outside network is blocked, so live pool numbers and wallets never differ between two runs.
Exit code of `diff` is 1 when any view differs.
"""
from __future__ import annotations

import difflib
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
PORT = 4199
BASE = f"http://127.0.0.1:{PORT}"
HASH_VIEWS = ["", "#faq", "#whitepaper", "#protocols", "#about"]
TABS = ["Stake", "Back the pool", "File a claim", "Collect payout"]


def serve() -> subprocess.Popen:
    cmd = ["npx", "vite", "preview", "--host", "127.0.0.1", "--port", str(PORT), "--strictPort"]
    proc = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(60):
        try:
            urllib.request.urlopen(BASE, timeout=1)
            return proc
        except OSError:
            time.sleep(0.5)
    proc.terminate()
    raise RuntimeError(f"vite preview did not start on {BASE}")


def local_only(route) -> None:
    if route.request.url.startswith(BASE):
        route.continue_()
    else:
        route.abort()


def body_html(page) -> str:
    page.wait_for_timeout(1500)
    return page.evaluate("document.body.innerHTML")


def record(page) -> dict[str, str]:
    views: dict[str, str] = {}
    for h in HASH_VIEWS:
        page.goto(f"{BASE}/{h}")
        views[f"view{h or '#main'}"] = body_html(page)
    page.goto(f"{BASE}/")
    for tab in TABS:
        page.locator(".tabs").get_by_role("button", name=tab, exact=True).click()
        views[f"tab:{tab}"] = body_html(page)
    views["title"] = page.title()
    return views


def snapshot(out: Path) -> None:
    proc = serve()
    try:
        with sync_playwright() as p:
            page = p.chromium.launch().new_page()
            page.route("**/*", local_only)
            out.write_text(json.dumps(record(page), indent=1))
    finally:
        proc.terminate()
    print(f"written {out}")


def diff(a: Path, b: Path) -> int:
    before, after = json.loads(a.read_text()), json.loads(b.read_text())
    changed = [k for k in sorted(before.keys() | after.keys()) if before.get(k) != after.get(k)]
    for k in changed:
        old, new = before.get(k, "").split("><"), after.get(k, "").split("><")
        print(f"--- {k}")
        print("\n".join(list(difflib.unified_diff(old, new, lineterm="", n=1))[:40]))
    print(f"{len(changed)} of {len(before)} views differ")
    return 1 if changed else 0


if __name__ == "__main__":
    if sys.argv[1] == "snapshot":
        snapshot(Path(sys.argv[2]))
    else:
        sys.exit(diff(Path(sys.argv[2]), Path(sys.argv[3])))
