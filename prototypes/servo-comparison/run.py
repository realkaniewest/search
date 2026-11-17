# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Run the same local readiness fixture in an existing ServoShell binary."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import ProxyHandler, Request, build_opener


DEFAULT_BASE = "http://127.0.0.1:43130"
READY_EVENT = "scenario-ready"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def checked_base(value: str) -> str:
    parts = urlsplit(value)
    if (
        parts.scheme != "http"
        or parts.hostname != "127.0.0.1"
        or parts.port is None
        or parts.username is not None
        or parts.password is not None
        or parts.path not in ("", "/")
        or parts.query
        or parts.fragment
    ):
        raise ValueError("fixture base must be http://127.0.0.1:<port>")
    return f"http://127.0.0.1:{parts.port}"


def request(url: str, *, method: str = "GET", headers: dict[str, str] | None = None) -> bytes:
    opener = build_opener(ProxyHandler({}))
    with opener.open(Request(url, data=b"" if method == "POST" else None, method=method, headers=headers or {}), timeout=5) as response:
        if response.status != 200:
            raise RuntimeError(f"fixture returned HTTP {response.status}: {url}")
        return response.read()


def has_ready_event(raw: bytes) -> bool:
    events = json.loads(raw)
    if not isinstance(events, list):
        raise ValueError("fixture /events must return a JSON array")
    return any(isinstance(item, dict) and item.get("event") == READY_EVENT for item in events)


def png_info(path: Path) -> tuple[int, int, str]:
    data = path.read_bytes()
    if len(data) < 24 or data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR":
        raise ValueError("ServoShell did not write a PNG with an IHDR header")
    width, height = struct.unpack(">II", data[16:24])
    if width == 0 or height == 0:
        raise ValueError("ServoShell PNG has invalid dimensions")
    return width, height, hashlib.sha256(data).hexdigest()


def run(binary: Path, base: str, timeout: int) -> None:
    if not binary.is_file():
        raise ValueError(f"ServoShell binary not found: {binary}")
    base = checked_base(base)
    request(f"{base}/reset", method="POST", headers={"X-Search-Fixture-Control": "1"})

    with tempfile.TemporaryDirectory(prefix="search-servo-comparison-") as temp:
        root = Path(temp)
        screenshot = root / "screen.png"
        profile = root / "profile"
        command = [
            str(binary.resolve()),
            "--headless",
            "--exit",
            f"--output={screenshot}",
            f"--config-dir={profile}",
            f"{base}/scenario.html",
        ]
        try:
            result = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                errors="replace",
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as exc:
            raise RuntimeError(f"ServoShell exceeded {timeout}s") from exc
        if result.returncode != 0:
            tail = (result.stderr or result.stdout)[-1000:].strip()
            raise RuntimeError(f"ServoShell exited {result.returncode}; log tail: {tail}")
        width, height, digest = png_info(screenshot)

        ready = False
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if has_ready_event(request(f"{base}/events")):
                ready = True
                break
            time.sleep(0.1)
        if not ready:
            raise RuntimeError("fixture did not record scenario-ready")

        print(f"PASS: ServoShell exit=0, scenario-ready, PNG={width}x{height}, sha256={digest}")
        print("LIMIT: headless local readiness only; no clicks, popups, uploads, downloads, media, GUI or speed comparison")
        print("LIMIT: supplied ServoShell binary/version was not verified by this runner")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="path to an existing official ServoShell binary")
    parser.add_argument("--base-url", default=DEFAULT_BASE, help="loopback fixture base URL")
    parser.add_argument("--timeout", type=int, default=90, help="process timeout in seconds")
    args = parser.parse_args()
    if args.timeout < 1 or args.timeout > 600:
        parser.error("--timeout must be between 1 and 600 seconds")
    try:
        run(args.binary, args.base_url, args.timeout)
    except (ValueError, RuntimeError, HTTPError, URLError, OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
