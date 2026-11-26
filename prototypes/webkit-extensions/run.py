"""Run the WebKit extension probe against a loopback-only fixture."""

import argparse
import json
import os
import stat

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from subprocess import run
from tempfile import TemporaryDirectory, mkdtemp
from shutil import rmtree
from threading import Thread
from uuid import uuid4


CHROME = Path(__file__).resolve().parents[2] / "apps/search/src/chrome.html"


def stage_extension(source, parent):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    remaining_entries = 512
    remaining_bytes = 16 * 1024 * 1024

    def copy_directory(directory, entries, destination, depth):
        nonlocal remaining_entries, remaining_bytes
        if depth > 12:
            raise ValueError("extension directory is too deep")
        for name in entries:
            remaining_entries -= 1
            if remaining_entries < 0:
                raise ValueError("extension has too many resources")
            descriptor = os.open(name.name, flags, dir_fd=directory)
            try:
                mode = os.fstat(descriptor).st_mode
                target = destination / name.name
                if stat.S_ISDIR(mode):
                    target.mkdir()
                    scan(descriptor, target, depth + 1)
                elif stat.S_ISREG(mode):
                    with os.fdopen(os.dup(descriptor), "rb") as resource, target.open("xb") as output:
                        while chunk := resource.read(min(65536, remaining_bytes + 1)):
                            remaining_bytes -= len(chunk)
                            if remaining_bytes < 0:
                                raise ValueError("extension resources are too large")
                            output.write(chunk)
                else:
                    raise ValueError("extension resource must be a regular file")
            finally:
                os.close(descriptor)

    def scan(descriptor, destination, depth):
        with os.scandir(descriptor) as entries:
            copy_directory(descriptor, entries, destination, depth)

    staging = Path(mkdtemp(prefix="staged-", dir=parent))
    try:
        directory = os.open(source, flags | os.O_DIRECTORY)
        try:
            scan(directory, staging, 0)
        finally:
            os.close(directory)
        return staging
    except BaseException:
        rmtree(staging)
        raise


def staged_fixture(parent):
    source = parent / "chosen-extension"
    source.mkdir()
    (source / "manifest.json").write_text(json.dumps({
        "manifest_version": 3, "name": "Search fixture", "version": "1.0",
        "permissions": ["storage"], "optional_permissions": ["tabs"],
        "optional_host_permissions": ["https://example.test/*"],
        "content_scripts": [{
            "matches": ["http://localhost/*", "http://127.0.0.1/*"], "js": ["marker.js"],
        }],
    }), encoding="utf-8")
    script = source / "marker.js"
    script.write_text("""
if (typeof window.ipc === 'undefined')
  document.documentElement.dataset.searchExtensionProbe = 'ran';
else window.ipc.postMessage('extension-boundary-fixture');
""", encoding="utf-8")
    link = source / "unapproved.js"
    outside = parent / "outside.js"
    outside.write_text("outside the selected directory", encoding="utf-8")
    link.symlink_to(outside)
    try:
        stage_extension(source, parent)
    except OSError:
        pass
    else:
        raise AssertionError("staging accepted a symbolic link")
    link.unlink()
    staged = stage_extension(source, parent)
    (source / "manifest.json").write_text("{}", encoding="utf-8")
    script.write_text("throw new Error('source changed');", encoding="utf-8")
    return staged


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
    parser = argparse.ArgumentParser()
    parser.add_argument("--wry-binary", type=Path)
    options = parser.parse_args()
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
            if options.wry_binary:
                result = run([options.wry_binary.resolve(), f"http://localhost:{port}/",
                              profile, str(alternate.server_port), staged_fixture(Path(temp))], timeout=30, check=False)
            else:
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
