# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Local attachment fixture for comparing browser engines."""

from http.server import BaseHTTPRequestHandler, HTTPServer


PAYLOAD = b"Search Servo download probe\n"
PAGE = b'''<!doctype html>
<meta charset="utf-8">
<title>Search download fixture</title>
<h1>Search download fixture</h1>
<a id="download" href="/download">Download test file</a>
<script>setTimeout(() => document.getElementById('download').click(), 1000)</script>
'''


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(2)

    def do_GET(self):
        if self.path == "/page":
            body = PAGE
            content_type = "text/html; charset=utf-8"
            attachment = False
        elif self.path == "/download":
            body = PAYLOAD
            content_type = "application/octet-stream"
            attachment = True
        else:
            body = b""
            content_type = "text/plain"
            attachment = False

        self.send_response(200 if self.path in {"/page", "/download"} else 404)
        self.send_header("Content-Type", content_type)
        if attachment:
            self.send_header(
                "Content-Disposition",
                'attachment; filename="search-servo-v070-probe.txt"',
            )
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    server = HTTPServer(("127.0.0.1", 43129), Handler)
    print("READY http://127.0.0.1:43129/page", flush=True)
    server.serve_forever()
