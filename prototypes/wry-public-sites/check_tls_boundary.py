"""Check a local WKWebView TLS rejection without changing system trust."""

import argparse
import http.server
import ssl
import subprocess
import tempfile
import threading
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    options = parser.parse_args()
    alerts = []
    requests = []

    class Server(http.server.ThreadingHTTPServer):
        def get_request(self):
            connection, address = super().get_request()
            try:
                connection.settimeout(2)
                return tls.wrap_socket(connection, server_side=True), address
            except OSError as error:
                alerts.append(getattr(error, "reason", None))
                connection.close()
                raise

    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"<title>Unexpected TLS fixture</title>")

    with tempfile.TemporaryDirectory(prefix="search-tls-") as directory:
        certificate = Path(directory) / "cert.pem"
        key = Path(directory) / "key.pem"
        subprocess.run(
            [
                "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                "-keyout", str(key), "-out", str(certificate),
            ],
            check=True,
            capture_output=True,
        )
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(certificate, key)
        with Server(("127.0.0.1", 0), Fixture) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                result = subprocess.run(
                    [str(options.binary.resolve()), f"https://127.0.0.1:{server.server_port}/"],
                    timeout=15,
                    check=False,
                )
            finally:
                server.shutdown()
                thread.join()

    certificate_alerts = {"SSLV3_ALERT_CERTIFICATE_UNKNOWN", "TLSV1_ALERT_UNKNOWN_CA"}
    print(f"TLS alerts={alerts}; HTTP requests={requests}; window exit={result.returncode}")
    if result.returncode or requests or not certificate_alerts.intersection(alerts):
        return 1
    print("PASS: WKWebView rejected the certificate before sending HTTP")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
