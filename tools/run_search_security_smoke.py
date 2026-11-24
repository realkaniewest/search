"""Exercise the first Search window against a local untrusted page."""

import http.server
import json
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


HOST = "127.0.0.1"
PORT = 43127
MEDIA_FRAME_PORT = 43128
TITLE = "Search security fixture ready"

MEDIA_FRAME = b"""<!doctype html><script>
(async () => {
    const geolocation = %%GEOLOCATION%% && navigator.geolocation
        ? new Promise(resolve => navigator.geolocation.getCurrentPosition(
            () => resolve('granted'),
            error => resolve(error.code === 1 ? 'denied' : 'unavailable'),
            {timeout: 6000}))
        : Promise.resolve('unavailable');
    let outcome = 'unavailable';
    if (navigator.mediaDevices?.getUserMedia) {
        try {
            const stream = await navigator.mediaDevices.getUserMedia({audio: true});
            stream.getTracks().forEach(track => track.stop());
            outcome = 'granted';
        } catch (error) {
            outcome = error.name === 'NotAllowedError' || error.name === 'SecurityError'
                ? 'denied' : error.name;
        }
    }
    parent.postMessage({microphone: outcome, geolocation: await geolocation}, "http://127.0.0.1:43127");
})();
</script>"""


def fixture_page(file_url, child_url=None):
    child_origin = child_url.rsplit("/", 1)[0] if child_url else None
    return f"""<!doctype html><html><head><meta charset='utf-8'><title>{TITLE}</title></head>
<body><h1>Search security fixture</h1><script>
(async () => {{
  const result = {{ipc: typeof window.ipc !== 'undefined', popup: false,
    camera: 'unavailable', microphone: 'unavailable', geolocation: 'unavailable',
    previousCookie: document.cookie, previousStorage: localStorage.getItem('search_probe')}};
  document.cookie = 'search_probe=stored; SameSite=Lax';
  localStorage.setItem('search_probe', 'stored');
  result.wroteCookie = document.cookie.includes('search_probe=stored');
  result.wroteStorage = localStorage.getItem('search_probe') === 'stored';
  try {{ result.popup = !!window.open('https://blocked.invalid/', '_blank'); }} catch (_) {{}}
  const link = document.createElement('a'); link.href = '/download'; link.download = 'blocked.txt';
  document.body.append(link); link.click();
  if ({json.dumps(child_url is not None)}) {{
    const frameResult = await new Promise(resolve => {{
      const frame = document.createElement('iframe');
      function finish(outcome) {{
        window.removeEventListener('message', receive);
        clearTimeout(timeout);
        frame.remove();
        resolve(outcome);
      }}
      function receive(event) {{
        if (event.origin === {json.dumps(child_origin)} && event.source === frame.contentWindow &&
            typeof event.data?.microphone === 'string' &&
            typeof event.data?.geolocation === 'string') finish(event.data);
      }}
      window.addEventListener('message', receive);
      const timeout = setTimeout(() => finish({{microphone: 'timeout', geolocation: 'timeout'}}), 6500);
      frame.allow = 'microphone *; geolocation *';
      frame.src = {json.dumps(child_url)};
      document.body.append(frame);
    }});
    result.frameMicrophone = frameResult.microphone;
    result.frameGeolocation = frameResult.geolocation;
  }}
  if (navigator.mediaDevices?.getUserMedia) {{
    async function checkMedia(constraints) {{
      return Promise.race([
        navigator.mediaDevices.getUserMedia(constraints).then(stream => {{
          stream.getTracks().forEach(track => track.stop());
          return 'granted';
        }}, error => error.name === 'NotAllowedError' || error.name === 'SecurityError'
          ? 'denied' : 'media-error:' + error.name),
        new Promise(resolve => setTimeout(() => resolve('timeout'), 6000))
      ]);
    }}
    result.camera = await checkMedia({{video: true}});
    result.microphone = await checkMedia({{audio: true}});
  }}
  if (navigator.geolocation) {{
    result.geolocation = await Promise.race([
      new Promise(resolve => navigator.geolocation.getCurrentPosition(
        () => resolve('granted'),
        error => resolve(error.code === 1 ? 'denied'
          : error.code === 2 ? 'unavailable' : error.code === 3 ? 'timeout' : 'error'),
        {{timeout: 6000}}
      )),
      new Promise(resolve => setTimeout(() => resolve('timeout'), 6000))
    ]);
  }}
  // Both targets are test-owned. The scheme has no registered application.
  for (const [name, target] of [
    ['fileNavigationStayedOnFixture', {json.dumps(file_url)}],
    ['customSchemeStayedOnFixture', 'search-smoke-unregistered://local/canary'],
  ]) {{
    const before = location.href;
    try {{ location.assign(target); }} catch (_) {{}}
    await new Promise(resolve => setTimeout(resolve, 250));
    result[name] = location.href === before && document.title === {json.dumps(TITLE)};
  }}
  await fetch('/results', {{method:'POST', headers:{{'content-type':'application/json'}}, body:JSON.stringify(result)}});
  setTimeout(() => {{ location.href = 'https://blocked.invalid/'; }}, 100);
}})();
</script></body></html>""".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    page = b""
    results = []
    downloads = 0
    frame_geolocation = False

    def do_GET(self):
        if self.path == "/":
            data, kind = type(self).page, "text/html; charset=utf-8"
        elif self.path == "/media-frame":
            data = MEDIA_FRAME.replace(b"%%GEOLOCATION%%", b"true" if self.frame_geolocation else b"false")
            kind = "text/html; charset=utf-8"
        elif self.path == "/download":
            type(self).downloads += 1
            data, kind = b"blocked download", "application/octet-stream"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        if self.path != "/results":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if length > 4096:
            self.send_error(413)
            return
        try:
            type(self).results.append(json.loads(self.rfile.read(length)))
        except (ValueError, UnicodeDecodeError):
            self.send_error(400)
            return
        self.send_response(204)
        self.end_headers()

    def log_message(self, _format, *_args):
        pass


def main():
    command = sys.argv[1:]
    require_media_origin = False
    require_geolocation_origin = False
    while command and command[0] in ("--require-media-origin", "--require-geolocation-origin"):
        flag = command.pop(0)
        require_media_origin |= flag == "--require-media-origin"
        require_geolocation_origin |= flag == "--require-geolocation-origin"
    if not command:
        print("Usage: run_search_security_smoke.py [--require-media-origin] [--require-geolocation-origin] <binary> [wrapper args]", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory(prefix="search-navigation-smoke-") as directory:
        canary = Path(directory) / "canary.html"
        canary.write_text("<title>Search file navigation canary</title>", encoding="utf-8")
        return run_fixture(command + ["--security-smoke"], canary.as_uri(), require_media_origin, require_geolocation_origin)


def run_fixture(command, file_url, require_media_origin, require_geolocation_origin):
    if require_media_origin or require_geolocation_origin:
        with http.server.ThreadingHTTPServer((HOST, MEDIA_FRAME_PORT), Handler) as alternate:
            thread = threading.Thread(target=alternate.serve_forever, daemon=True)
            thread.start()
            try:
                return run_fixtures(command, file_url, require_media_origin, require_geolocation_origin)
            finally:
                alternate.shutdown()
                thread.join()
    return run_fixtures(command, file_url, require_media_origin, require_geolocation_origin)


def run_fixtures(command, file_url, require_media_origin, require_geolocation_origin):
    server = http.server.ThreadingHTTPServer((HOST, PORT), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        verify_origins = require_media_origin or require_geolocation_origin
        Handler.frame_geolocation = require_geolocation_origin
        for attempt in range(3 if verify_origins else 2):
            # WebKit can reuse an earlier denial within a view. A fresh process
            # keeps the first request attributable to the intended frame.
            child_frame = verify_origins and attempt > 0
            alternate = verify_origins and attempt == 2
            expected_frame = "child" if child_frame else "main"
            expected_origin = "alternate" if alternate else "primary"
            child_url = f"http://{HOST}:{MEDIA_FRAME_PORT if alternate else PORT}/media-frame"
            Handler.page = fixture_page(file_url, child_url if child_frame else None)
            try:
                run = subprocess.run(command, capture_output=True, text=True, timeout=65, check=False)
            except subprocess.TimeoutExpired as error:
                for stream in (error.stdout, error.stderr):
                    if stream:
                        print(stream.decode(errors="replace"), file=sys.stderr, end="")
                print(f"FAIL: Search security smoke timed out; reports={Handler.results!r}", file=sys.stderr)
                return 1
            print(run.stdout, end="")
            print(run.stderr, end="", file=sys.stderr)
            results = Handler.results[attempt * 2:]
            if run.returncode != 0 or "PASS: Search security boundary checks" not in run.stdout:
                print(f"Fixture result: {results!r}", file=sys.stderr)
                print("FAIL: Search did not confirm boundary checks", file=sys.stderr)
                return 1
            if require_media_origin and (
                run.stdout.count(f"SECURITY: media_origin=primary frame={expected_frame} frame_origin={expected_origin}") < 2
                or "SECURITY: media_origin=unexpected" in run.stdout
                or "frame_origin=unexpected" in run.stdout
            ):
                print(f"FAIL: WebKit did not report the expected {expected_frame}-frame media origin", file=sys.stderr)
                return 1
            if require_geolocation_origin and (
                run.stdout.count(f"SECURITY: geolocation_origin=primary frame={expected_frame} frame_origin={expected_origin}") < 2
                or "SECURITY: geolocation_origin=unexpected" in run.stdout
            ):
                print("FAIL: WebKit did not report the expected geolocation origin", file=sys.stderr)
                return 1
            if len(results) != 2:
                print(f"FAIL: expected two fixture tabs, received {len(results)}", file=sys.stderr)
                return 1
            for result in results:
                if not isinstance(result, dict) or result.get("ipc") is not False or result.get("popup") is not False:
                    print(f"FAIL: fixture boundary result {result!r}", file=sys.stderr)
                    return 1
                for check in ("fileNavigationStayedOnFixture", "customSchemeStayedOnFixture"):
                    if result.get(check) is not True:
                        print(f"FAIL: {check} unverified: {result!r}", file=sys.stderr)
                        return 1
                if child_frame and require_geolocation_origin and result.get("frameGeolocation") != "denied":
                    print(f"FAIL: unexpected child-frame geolocation result: {result.get('frameGeolocation')}", file=sys.stderr)
                    return 1
                if child_frame and require_media_origin and result.get("frameMicrophone") != "denied":
                    print(f"FAIL: unexpected child-frame media result: {result.get('frameMicrophone')}", file=sys.stderr)
                    return 1
                permission_results = {
                    name: result.get(name) for name in ("camera", "microphone", "geolocation")
                }
                if require_geolocation_origin and permission_results["geolocation"] != "denied":
                    print("FAIL: native geolocation denial did not reach the page", file=sys.stderr)
                    return 1
                print(f"Permission results: {permission_results}")
                if (
                    any(
                        value not in ("denied", "unavailable")
                        and not (name != "geolocation" and isinstance(value, str) and value.startswith("media-error:"))
                        for name, value in permission_results.items()
                    )
                    or "denied" not in permission_results.values()
                ):
                    print(f"FAIL: sensitive permission denial unverified: {result!r}", file=sys.stderr)
                    return 1
                if result.get("wroteCookie") is not True or result.get("wroteStorage") is not True:
                    print(f"FAIL: fixture did not seed temporary data: {result!r}", file=sys.stderr)
                    return 1
                if (
                    result.get("previousCookie") != ""
                    or "previousStorage" not in result
                    or result["previousStorage"] is not None
                ):
                    print(f"FAIL: temporary data visible from another tab or process: {result!r}", file=sys.stderr)
                    return 1
        print("PASS: file URL and unregistered scheme left the local fixture loaded")
        print(f"PASS: fixture saw no IPC/popup/permission grant or retained cookie/localStorage; download requests={Handler.downloads}")
        return 0
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    raise SystemExit(main())
