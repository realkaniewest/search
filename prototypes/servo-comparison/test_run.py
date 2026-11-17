# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Unit and local integration checks without a Servo download."""

from __future__ import annotations

import contextlib
import base64
import io
import json
import os
import tempfile
import threading
import unittest
from unittest.mock import patch
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from run import checked_base, has_ready_event, png_info, request, run


class FixtureHandler(BaseHTTPRequestHandler):
    events: list[dict[str, str]] = []

    def log_message(self, *_args: object) -> None:
        pass

    def do_GET(self) -> None:
        if self.path != "/events":
            self.send_error(404)
            return
        data = json.dumps(type(self).events).encode()
        self.send_response(200)
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self) -> None:
        if self.path == "/reset":
            type(self).events = []
        elif self.path == "/event":
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            type(self).events.append(json.loads(raw))
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")


class RunnerTests(unittest.TestCase):
    def test_base_restricts_to_loopback_http(self) -> None:
        self.assertEqual(checked_base("http://127.0.0.1:43130"), "http://127.0.0.1:43130")
        for address in ("https://127.0.0.1:43130", "http://localhost:43130", "http://example.com:43130", "http://127.0.0.1:43130/other"):
            with self.subTest(address=address), self.assertRaises(ValueError):
                checked_base(address)

    def test_ready_requires_named_event(self) -> None:
        self.assertTrue(has_ready_event(b'[{"event":"scenario-ready"}]'))
        self.assertFalse(has_ready_event(b'[{"event":"request","path":"/scenario.html"}]'))
        with self.assertRaises(ValueError):
            has_ready_event(b'{"event":"scenario-ready"}')

    def test_loopback_request_ignores_proxy_environment(self) -> None:
        FixtureHandler.events = []
        server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with patch.dict(os.environ, {"HTTP_PROXY": "http://127.0.0.1:1", "http_proxy": "http://127.0.0.1:1", "NO_PROXY": "", "no_proxy": ""}):
                self.assertEqual(request(f"http://127.0.0.1:{server.server_port}/events"), b"[]")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    @unittest.skipIf(os.name == "nt", "mock executable uses a POSIX shebang")
    def test_complete_run(self) -> None:
        server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f"http://127.0.0.1:{server.server_port}"
        try:
            with tempfile.TemporaryDirectory() as temp:
                fake = Path(temp) / "fake-servo"
                fake.write_text(
                    "#!/usr/bin/env python3\n"
                    "import base64, pathlib, sys, urllib.request\n"
                    "args = sys.argv[1:]\n"
                    "out = pathlib.Path(next(x.split('=', 1)[1] for x in args if x.startswith('--output=')))\n"
                    "out.write_bytes(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l8kAAAAASUVORK5CYII='))\n"
                    "base = args[-1].removesuffix('/scenario.html')\n"
                    "urllib.request.urlopen(urllib.request.Request(base + '/event', data=b'{\"event\":\"scenario-ready\"}', method='POST'))\n",
                    encoding="utf-8",
                )
                fake.chmod(0o755)
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    run(fake, base, 10)
                self.assertIn("PASS: ServoShell exit=0, scenario-ready", output.getvalue())
                self.assertTrue(FixtureHandler.events)
                # The profile and screenshot live in a TemporaryDirectory and are gone after run.
                self.assertEqual(list(Path(temp).glob("profile")), [])
                sample = Path(temp) / "sample.png"
                sample.write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l8kAAAAASUVORK5CYII="))
                width, height, digest = png_info(sample)
                self.assertEqual((width, height), (1, 1))
                self.assertEqual(len(digest), 64)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
