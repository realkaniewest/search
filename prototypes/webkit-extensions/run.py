"""Run the WebKit extension probe against a loopback-only fixture."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from subprocess import run
from tempfile import TemporaryDirectory
from threading import Thread
from uuid import uuid4


CHROME = Path(__file__).resolve().parents[2] / "apps/search/src/chrome.html"


class Fixture(BaseHTTPRequestHandler):
    alternate_port: int

    def do_GET(self):
        port = self.server.server_port
        if self.path == "/chrome":
            body = CHROME.read_bytes()
        elif self.path == "/frames":
            body = f"""<!doctype html><title>Search frame fixture</title>
<script>
window.frameReports = {{}};
addEventListener('message', event => {{
    for (const frame of document.querySelectorAll('iframe')) {{
        if (event.source === frame.contentWindow && event.origin === new URL(frame.src).origin &&
            typeof event.data === 'boolean') window.frameReports[frame.id] = event.data;
    }}
}});
</script>
<iframe id="allowed" src="http://localhost:{port}/report"></iframe>
<iframe id="denied" src="http://127.0.0.1:{port}/report"></iframe>
<iframe id="alternatePort" src="http://localhost:{self.alternate_port}/report"></iframe>""".encode()
        elif self.path == "/report":
            body = b"""<!doctype html><title>Search frame report</title>
<script>
addEventListener('load', () => setTimeout(() => {
    parent.postMessage(document.documentElement.dataset.searchExtensionProbe === 'ran', '*');
}, 150));
</script>"""
        else:
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
    with (
        TemporaryDirectory(prefix="search-extension-probe-") as temp,
        ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as server,
        ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as alternate,
    ):
        Fixture.alternate_port = alternate.server_port
        servers = (server, alternate)
        threads = [Thread(target=httpd.serve_forever, daemon=True) for httpd in servers]
        for thread in threads:
            thread.start()
        port = server.server_port
        try:
            state = str(Path(temp) / "permissions.json")
            arguments = [
                *probe, f"http://localhost:{port}/", f"http://127.0.0.1:{port}/", profile,
                f"http://localhost:{alternate.server_port}/",
            ]
            for phase in ("initial", "restart", "revoked"):
                result = run([*arguments, phase, state], timeout=30, check=False)
                if result.returncode:
                    break
        finally:
            for httpd, thread in zip(servers, threads):
                httpd.shutdown()
                thread.join()
            cleanup = run([*probe, "--cleanup", profile], timeout=20, check=False)
    raise SystemExit(result.returncode or cleanup.returncode)
