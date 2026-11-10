# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Serve the fixed Servo fixture and record request methods without tokens."""

import argparse
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


parser = argparse.ArgumentParser()
parser.add_argument("--root", type=Path, required=True)
parser.add_argument("--log", type=Path, required=True)
options = parser.parse_args()


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(options.root), **kwargs)

    def log_request(self, code="-", size="-"):
        with options.log.open("a", encoding="utf-8") as log:
            log.write(f"{self.command} {self.path} {code}\n")


ThreadingHTTPServer(("127.0.0.1", 43132), Handler).serve_forever()
