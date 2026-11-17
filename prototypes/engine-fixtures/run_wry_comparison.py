# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Run the Wry comparison probe against a fresh loopback fixture server."""

import argparse
import subprocess
import sys
from threading import Thread
from pathlib import Path

from comparison_server import ComparisonServer


def run(binary: Path, timeout: int) -> None:
    if not binary.is_file():
        raise ValueError(f"Wry probe not found: {binary}")
    with ComparisonServer(("127.0.0.1", 0)) as server:
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        url = f"http://127.0.0.1:{server.server_address[1]}/scenario.html"
        try:
            result = subprocess.run(
                [str(binary.resolve()), url],
                capture_output=True,
                text=True,
                errors="replace",
                timeout=timeout,
                check=False,
            )
        finally:
            server.shutdown()
            thread.join(timeout=3)
        if result.returncode != 0:
            raise RuntimeError(f"Wry exited {result.returncode}; stderr: {result.stderr[-1000:]}")
        if "PASS: local fixture changed the document title to Search comparison ready" not in result.stdout:
            raise RuntimeError(f"Wry readiness marker missing; stdout: {result.stdout[-1000:]}")
        events = server.snapshot()
        if {"event": "request", "path": "/scenario.html"} not in events or {"event": "scenario-ready"} not in events:
            raise RuntimeError(f"fixture events missing: {events}")
        print(f"PASS: Wry title and fixture events observed at {url}")
        print("LIMIT: local readiness only; no clicks, popups, uploads, downloads, media, visual pixels or speed comparison")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=45)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 180:
        parser.error("--timeout must be between 1 and 180 seconds")
    try:
        run(args.binary, args.timeout)
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
