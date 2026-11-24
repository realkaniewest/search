"""Run the WebKit extension probe against a loopback-only fixture."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from subprocess import run
from threading import Thread
from uuid import uuid4


class Fixture(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"<!doctype html><title>Search WebKit extension fixture</title>"
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    profile = str(uuid4())
    probe = ["swift", str(Path(__file__).with_name("probe.swift"))]
    with ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as server:
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        port = server.server_port
        try:
            result = run(
                [*probe, f"http://localhost:{port}/", f"http://127.0.0.1:{port}/", profile],
                timeout=30,
                check=False,
            )
        finally:
            server.shutdown()
            thread.join()
            cleanup = run([*probe, "--cleanup", profile], timeout=20, check=False)
    raise SystemExit(result.returncode or cleanup.returncode)
