"""Exercise the first Search window against a local untrusted page."""

import http.server
import json
import subprocess
import sys
import threading


HOST = "127.0.0.1"
PORT = 43127
TITLE = "Search security fixture ready"
PAGE = f"""<!doctype html><html><head><meta charset='utf-8'><title>{TITLE}</title></head>
<body><h1>Search security fixture</h1><script>
(async () => {{
  const result = {{ipc: typeof window.ipc !== 'undefined', popup: false,
    camera: 'unavailable', microphone: 'unavailable', geolocation: 'unavailable',
    previousCookie: document.cookie, previousStorage: localStorage.getItem('search_probe')}};
  document.cookie = 'search_probe=stored; SameSite=Lax';
  localStorage.setItem('search_probe', 'stored');
  try {{ result.popup = !!window.open('https://blocked.invalid/', '_blank'); }} catch (_) {{}}
  const link = document.createElement('a'); link.href = '/download'; link.download = 'blocked.txt';
  document.body.append(link); link.click();
  if (navigator.mediaDevices?.getUserMedia) {{
    async function checkMedia(constraints) {{
      return Promise.race([
        navigator.mediaDevices.getUserMedia(constraints).then(stream => {{
          stream.getTracks().forEach(track => track.stop());
          return 'granted';
        }}, error => error.name === 'NotAllowedError' || error.name === 'SecurityError'
          ? 'denied' : 'media-error:' + error.name),
        new Promise(resolve => setTimeout(() => resolve('timeout'), 2500))
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
        {{timeout: 2500}}
      )),
      new Promise(resolve => setTimeout(() => resolve('timeout'), 2500))
    ]);
  }}
  await fetch('/results', {{method:'POST', headers:{{'content-type':'application/json'}}, body:JSON.stringify(result)}});
  setTimeout(() => {{ location.href = 'https://blocked.invalid/'; }}, 100);
}})();
</script></body></html>""".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    results = []
    downloads = 0

    def do_GET(self):
        if self.path == "/":
            data, kind = PAGE, "text/html; charset=utf-8"
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
    if len(sys.argv) < 2:
        print("Usage: run_search_security_smoke.py <binary> [wrapper args]", file=sys.stderr)
        return 2
    server = http.server.ThreadingHTTPServer((HOST, PORT), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        command = sys.argv[1:] + ["--security-smoke"]
        for attempt in (1, 2):
            try:
                run = subprocess.run(command, capture_output=True, text=True, timeout=50, check=False)
            except subprocess.TimeoutExpired:
                print("FAIL: Search security smoke timed out", file=sys.stderr)
                return 1
            print(run.stdout, end="")
            print(run.stderr, end="", file=sys.stderr)
            result = Handler.results[-1] if len(Handler.results) == attempt else None
            if run.returncode != 0 or "PASS: Search security boundary checks" not in run.stdout:
                print(f"Fixture result: {result!r}", file=sys.stderr)
                print("FAIL: Search did not confirm boundary checks", file=sys.stderr)
                return 1
            if not isinstance(result, dict) or result.get("ipc") is not False or result.get("popup") is not False:
                print(f"FAIL: fixture boundary result {result!r}", file=sys.stderr)
                return 1
            permission_results = {
                name: result.get(name) for name in ("camera", "microphone", "geolocation")
            }
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
            if result.get("previousCookie") or result.get("previousStorage") is not None:
                print(f"FAIL: temporary data survived process start: {result!r}", file=sys.stderr)
                return 1
        print(f"PASS: fixture saw no IPC/popup/permission grant or retained cookie/localStorage; download requests={Handler.downloads}")
        return 0
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    raise SystemExit(main())
