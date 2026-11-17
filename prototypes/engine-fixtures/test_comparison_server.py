# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Route and event checks for the loopback comparison fixture."""

from hashlib import sha256
from http.client import HTTPConnection
from pathlib import Path
from threading import Thread
import json
import unittest

from comparison_server import ComparisonServer, DOWNLOAD_BODY, Handler, MAX_EVENTS, UPLOAD_BODY


class ComparisonServerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ComparisonServer(("127.0.0.1", 0), Handler)
        cls.thread = Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.port = cls.server.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=3)

    def setUp(self):
        self.request("POST", "/reset", b"")

    def request(self, method, path, body=None, headers=None):
        connection = HTTPConnection("127.0.0.1", self.port, timeout=3)
        try:
            source = ({"X-Search-Fixture-Control": "1"} if path == "/reset" else
                      {"Origin": f"http://127.0.0.1:{self.port}"}) if method == "POST" else {}
            source.update(headers or {})
            connection.request(method, path, body=body, headers=source)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def events(self):
        status, _, body = self.request("GET", "/events")
        self.assertEqual(status, 200)
        return json.loads(body)

    def test_pages_and_navigation_events(self):
        for path in ("/scenario.html", "/start", "/second", "/popup", "/upload-page", "/download-page", "/basic.html"):
            status, headers, body = self.request("GET", path)
            self.assertEqual(status, 200, path)
            self.assertIn("text/html", headers["Content-Type"])
            self.assertIn(b"<!doctype html>", body.lower())
        self.assertEqual([e["path"] for e in self.events() if e["event"] == "request"],
                         ["/scenario.html", "/start", "/second", "/popup", "/upload-page", "/download-page"])
        self.assertIn(b"Search comparison ready", self.request("GET", "/scenario.html")[2])
        status, _, _ = self.request("POST", "/event", b'{"name":"popup-clicked"}',
                                    {"Content-Type": "application/json"})
        self.assertEqual(status, 200)
        self.assertIn({"event": "popup-clicked"}, self.events())
        status, _, _ = self.request("POST", "/event", b'{"name":"other"}',
                                    {"Content-Type": "application/json"})
        self.assertEqual(status, 400)

    def test_download_and_upload_digest(self):
        self.assertEqual(Path(__file__).with_name("upload-sample.txt").read_bytes(), UPLOAD_BODY)
        status, headers, body = self.request("GET", "/download")
        self.assertEqual(status, 200)
        self.assertEqual(body, DOWNLOAD_BODY)
        self.assertIn("attachment", headers["Content-Disposition"])
        boundary = "search-fixture-boundary"
        multipart = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"fixture\"; filename=\"upload-sample.txt\"\r\n"
                     "Content-Type: text/plain\r\n\r\n").encode() + UPLOAD_BODY + f"\r\n--{boundary}--\r\n".encode()
        status, _, result = self.request("POST", "/upload", multipart,
                                         {"Content-Type": f"multipart/form-data; boundary={boundary}"})
        self.assertEqual(status, 200)
        digest = sha256(UPLOAD_BODY).hexdigest()
        self.assertIn(digest.encode(), result)
        self.assertIn({"event": "upload", "size": len(UPLOAD_BODY), "sha256": digest}, self.events())
        self.assertNotIn(UPLOAD_BODY.decode().strip(), json.dumps(self.events()))

    def test_no_arbitrary_paths_or_oversized_upload(self):
        for path in ("/../LICENSE", "/%2e%2e/LICENSE", "/prototypes/engine-fixtures/comparison_server.py"):
            status, _, body = self.request("GET", path)
            self.assertEqual(status, 404, path)
            self.assertEqual(body, b"not found\n")
        status, _, _ = self.request("POST", "/upload", b"x" * (64 * 1024 + 1),
                                    {"Content-Type": "multipart/form-data; boundary=x"})
        self.assertEqual(status, 413)
        self.assertEqual(self.events(), [])

    def test_rejects_foreign_sources_and_unexpected_files(self):
        status, _, _ = self.request("GET", "/events", headers={"Host": "attacker.example"})
        self.assertEqual(status, 403)
        status, _, _ = self.request("GET", "/scenario.html", headers={"Sec-Fetch-Site": "cross-site"})
        self.assertEqual(status, 403)
        status, headers, _ = self.request("GET", "/scenario.html")
        self.assertEqual(status, 200)
        self.assertIn("frame-ancestors 'none'", headers["Content-Security-Policy"])
        status, _, _ = self.request("POST", "/reset", b"", {"Origin": "https://attacker.example"})
        self.assertEqual(status, 403)
        status, _, _ = self.request("POST", "/event", b'{"name":"scenario-ready"}',
                                    {"Origin": "https://attacker.example", "Content-Type": "application/json"})
        self.assertEqual(status, 403)
        status, _, _ = self.request("POST", "/event", b'{"name":"scenario-ready"}',
                                    {"Content-Type": "text/plain"})
        self.assertEqual(status, 415)
        boundary = "search-fixture-boundary"
        personal = b"not the fixed sample\n"
        multipart = (f'--{boundary}\r\nContent-Disposition: form-data; name="fixture"; filename="other.txt"\r\n'
                     'Content-Type: text/plain\r\n\r\n').encode() + personal + f'\r\n--{boundary}--\r\n'.encode()
        status, _, body = self.request("POST", "/upload", multipart,
                                       {"Content-Type": f"multipart/form-data; boundary={boundary}"})
        self.assertEqual(status, 400)
        self.assertEqual(self.events(), [{"event": "request", "path": "/scenario.html"}])
        self.assertNotIn(sha256(personal).hexdigest().encode(), body)

    def test_accepts_servo_top_level_navigation_only(self):
        navigation = {"Sec-Fetch-Site": "cross-site", "Sec-Fetch-Mode": "navigate",
                      "Sec-Fetch-Dest": "document"}
        status, _, _ = self.request("GET", "/scenario.html", headers=navigation)
        self.assertEqual(status, 200)
        status, _, _ = self.request("GET", "/start", headers=navigation)
        self.assertEqual(status, 403)
        status, _, _ = self.request("POST", "/event", b'{"name":"scenario-ready"}',
                                    {**navigation, "Content-Type": "application/json"})
        self.assertEqual(status, 403)

    def test_event_limit(self):
        for _ in range(MAX_EVENTS):
            status, _, _ = self.request("POST", "/event", b'{"name":"scenario-ready"}',
                                        {"Content-Type": "application/json"})
            self.assertEqual(status, 200)
        status, _, _ = self.request("POST", "/event", b'{"name":"scenario-ready"}',
                                    {"Content-Type": "application/json"})
        self.assertEqual(status, 429)
        self.assertEqual(len(self.events()), MAX_EVENTS)


if __name__ == "__main__":
    unittest.main()
