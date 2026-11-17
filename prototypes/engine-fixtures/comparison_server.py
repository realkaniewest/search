# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Deterministic loopback-only pages for comparing browser engines."""

from argparse import ArgumentParser
from email import policy
from email.parser import BytesParser
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
from threading import Lock
from urllib.parse import urlsplit


HOST = "127.0.0.1"
PORT = 43130
MAX_POST_BYTES = 64 * 1024
MAX_EVENTS = 256
DOWNLOAD_NAME = "search-comparison.txt"
DOWNLOAD_BODY = b"Search engine comparison fixture v1\n"
UPLOAD_BODY = b"Search upload fixture v1\n"
ROOT = Path(__file__).resolve().parent

STYLE = "body{font:18px system-ui,sans-serif;max-width:48rem;margin:3rem auto;line-height:1.5}button,input{font:inherit}a,button{margin-right:1rem}pre{white-space:pre-wrap}"
EVENT_SCRIPT = """
<script>
const report = (name) => fetch('/event', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({name})}).catch(() => {});
</script>
"""


def page(title: str, body: str) -> bytes:
    return ("<!doctype html><html lang=\"ru\"><meta charset=\"utf-8\">"
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
            f"<title>{title}</title><style>{STYLE}</style>{EVENT_SCRIPT}<body>"
            f"<h1>{title}</h1>{body}</body></html>").encode("utf-8")


PAGES = {
    "/scenario.html": page("Search comparison: loading", """
        <p id="state">Проверка JavaScript и локального отчёта…</p>
        <script>report('scenario-ready').then(response => {
          if (!response || !response.ok) return;
          document.getElementById('state').textContent = 'Сценарий готов';
          document.body.dataset.ready = 'true';
          document.title = 'Search comparison ready';
        });</script>
    """),
    "/start": page("Search comparison: start", """
        <p>Стартовая страница. Нажмите ссылку, затем Назад и Вперёд в браузере.</p>
        <a id="next" href="/second">Перейти на вторую страницу</a>
        <button id="popup" type="button">Открыть новое окно</button>
        <p><a href="/upload-page">Отправка файла</a><a href="/download-page">Скачивание</a></p>
        <script>report('start-loaded');document.getElementById('popup').onclick=()=>{
          report('popup-clicked');window.open('/popup','_blank');};</script>
    """),
    "/second": page("Search comparison: second", """
        <p>Вторая страница. Вернитесь назад и снова вперёд.</p>
        <a href="/start">Вернуться на стартовую страницу</a>
        <script>report('second-loaded');</script>
    """),
    "/popup": page("Search comparison: popup", """
        <p>Если эта страница видна в новом окне или вкладке, открытие сработало.</p>
        <script>report('popup-loaded');</script>
    """),
    "/upload-page": page("Search comparison: upload", """
        <p>Выберите файл <code>upload-sample.txt</code> из этой папки и отправьте.</p>
        <form method="post" action="/upload" enctype="multipart/form-data">
          <input type="file" name="fixture" required><button type="submit">Отправить</button>
        </form>
    """),
    "/download-page": page("Search comparison: download", """
        <p>Нажмите ссылку и проверьте, создался ли файл <code>search-comparison.txt</code>.</p>
        <a href="/download">Скачать тестовый файл</a>
    """),
}


class ComparisonServer(ThreadingHTTPServer):
    def __init__(self, address, handler_class=None):
        super().__init__(address, handler_class or Handler)
        self.events = []
        self.event_lock = Lock()

    def record(self, event):
        with self.event_lock:
            if len(self.events) >= MAX_EVENTS:
                return False
            self.events.append(event)
            return True

    def snapshot(self):
        with self.event_lock:
            return list(self.events)

    def reset(self):
        with self.event_lock:
            self.events.clear()


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(3)

    def log_message(self, _format_string, *_args):
        # Do not print arbitrary paths, query strings or headers from test pages.
        pass

    def expected_origin(self):
        return f"http://{HOST}:{self.server.server_address[1]}"

    def allowed_host(self):
        return self.headers.get("Host") == self.expected_origin().removeprefix("http://")

    def allowed_fetch_site(self, path):
        site = self.headers.get("Sec-Fetch-Site")
        if site in (None, "none", "same-origin"):
            return True
        return (self.command in {"GET", "HEAD"} and path == "/scenario.html"
                and site == "cross-site"
                and self.headers.get("Sec-Fetch-Mode") == "navigate"
                and self.headers.get("Sec-Fetch-Dest") == "document")

    def allowed_post_source(self, path):
        origin = self.headers.get("Origin")
        referer = self.headers.get("Referer")
        expected = self.expected_origin()
        if origin is not None and origin != expected:
            return False
        if referer is not None:
            parts = urlsplit(referer)
            if f"{parts.scheme}://{parts.netloc}" != expected:
                return False
        if origin is not None or referer is not None:
            return True
        return path == "/reset" and self.headers.get("X-Search-Fixture-Control") == "1"

    def respond(self, code, body=b"", content_type="text/plain; charset=utf-8", *, download=False):
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'")
        if download:
            self.send_header("Content-Disposition", f'attachment; filename="{DOWNLOAD_NAME}"')
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        path = urlsplit(self.path).path
        if not self.allowed_host() or not self.allowed_fetch_site(path):
            return self.respond(403, b"invalid request source\n")
        if path == "/health":
            return self.respond(200, b"ready\n")
        if path == "/events":
            body = json.dumps(self.server.snapshot(), ensure_ascii=False).encode("utf-8")
            return self.respond(200, body, "application/json; charset=utf-8")
        if path == "/basic.html":
            return self.respond(200, (ROOT / "basic.html").read_bytes(), "text/html; charset=utf-8")
        if path == "/upload-sample.txt":
            return self.respond(200, UPLOAD_BODY, "text/plain; charset=utf-8")
        if path in PAGES:
            if self.command == "GET":
                if not self.server.record({"event": "request", "path": path}):
                    return self.respond(429, b"event limit reached\n")
            return self.respond(200, PAGES[path], "text/html; charset=utf-8")
        if path == "/download":
            if self.command == "GET":
                if not self.server.record({"event": "download-request"}):
                    return self.respond(429, b"event limit reached\n")
            return self.respond(200, DOWNLOAD_BODY, "application/octet-stream", download=True)
        self.respond(404, b"not found\n")

    def read_body(self):
        try:
            length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            return None
        if not 0 <= length <= MAX_POST_BYTES:
            return None
        return self.rfile.read(length)

    def do_POST(self):
        path = urlsplit(self.path).path
        if not self.allowed_host() or not self.allowed_fetch_site(path):
            return self.respond(403, b"invalid request source\n")
        if path not in {"/event", "/upload", "/reset"}:
            return self.respond(404, b"not found\n")
        if not self.allowed_post_source(path):
            return self.respond(403, b"invalid source\n")
        body = self.read_body()
        if body is None:
            return self.respond(413, b"invalid or oversized body\n")
        if path == "/reset":
            self.server.reset()
            return self.respond(200, b"reset\n")
        if path == "/event":
            if self.headers.get("Content-Type", "").split(";", 1)[0].strip().lower() != "application/json":
                return self.respond(415, b"json required\n")
            try:
                data = json.loads(body)
            except (ValueError, UnicodeDecodeError):
                return self.respond(400, b"invalid json\n")
            name = data.get("name") if isinstance(data, dict) else None
            if name not in {"scenario-ready", "start-loaded", "second-loaded", "popup-clicked", "popup-loaded"}:
                return self.respond(400, b"invalid event\n")
            if not self.server.record({"event": name}):
                return self.respond(429, b"event limit reached\n")
            return self.respond(200, b"recorded\n")
        content_type = self.headers.get("Content-Type", "")
        if not content_type.startswith("multipart/form-data;") or "\r" in content_type or "\n" in content_type:
            return self.respond(415, b"multipart required\n")
        try:
            parsed = BytesParser(policy=policy.default).parsebytes(
                b"MIME-Version: 1.0\r\nContent-Type: "
                + content_type.encode("ascii") + b"\r\n\r\n" + body
            )
            part = next(p for p in parsed.iter_parts() if p.get_param("name", header="content-disposition") == "fixture")
            file_body = part.get_payload(decode=True)
        except (UnicodeEncodeError, StopIteration, ValueError, TypeError):
            return self.respond(400, b"invalid multipart\n")
        if not isinstance(file_body, bytes):
            return self.respond(400, b"invalid file\n")
        if file_body != UPLOAD_BODY:
            return self.respond(400, b"only the fixed sample file is accepted\n")
        digest = sha256(file_body).hexdigest()
        if not self.server.record({"event": "upload", "size": len(file_body), "sha256": digest}):
            return self.respond(429, b"event limit reached\n")
        result = page("Search comparison: upload result", f"<p>Получено {len(file_body)} байт. SHA-256: {digest}</p>")
        return self.respond(200, result, "text/html; charset=utf-8")


def main():
    parser = ArgumentParser()
    parser.add_argument("--port", type=int, default=PORT)
    args = parser.parse_args()
    if not 0 <= args.port <= 65535:
        parser.error("port must be between 0 and 65535")
    with ComparisonServer((HOST, args.port)) as server:
        print(f"READY http://{HOST}:{server.server_address[1]}/start", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
