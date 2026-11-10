#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
workdir=$(mktemp -d)
server_pid=
servo_pid=
cleanup() {
  if [[ -n "$servo_pid" ]]; then kill "$servo_pid" 2>/dev/null || true; wait "$servo_pid" 2>/dev/null || true; fi
  if [[ -n "$server_pid" ]]; then kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; fi
  rm -rf "$workdir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

asset=servo-x86_64-linux-gnu.tar.gz
url="https://github.com/servo/servo/releases/download/v0.8.0/$asset"
expected=02a78892746e69bc39d471da2bb012fd8ce228e464cda5a017334d0e88c61de1
curl --fail --location --retry 3 --proto '=https' --tlsv1.2 "$url" --output "$workdir/$asset"
printf '%s  %s\n' "$expected" "$workdir/$asset" | sha256sum --check
tar -xzf "$workdir/$asset" -C "$workdir"

python3 -m http.server 43131 --bind 127.0.0.1 --directory "$root/prototypes/servo-linux-runtime" >"$workdir/server.log" 2>&1 &
server_pid=$!
for _ in {1..30}; do
  if curl --fail --silent --head http://127.0.0.1:43131/probe.html >/dev/null; then break; fi
  if ! kill -0 "$server_pid" 2>/dev/null; then cat "$workdir/server.log"; exit 1; fi
  sleep 0.2
done

"$workdir/servo/servoshell" --config-dir="$workdir/profile" http://127.0.0.1:43131/probe.html >"$workdir/servo.log" 2>&1 &
servo_pid=$!

pixels=0
for _ in {1..90}; do
  if ! kill -0 "$servo_pid" 2>/dev/null; then
    cat "$workdir/servo.log"
    echo 'FAIL: ServoShell exited before rendering the probe' >&2
    exit 1
  fi
  if import -window root "$workdir/screen.png" 2>/dev/null; then
    pixels=$(convert "$workdir/screen.png" -format %c histogram:info:- | awk '/#14B8A6/ { sum += $1 } END { print sum + 0 }')
    if (( pixels > 10000 )); then break; fi
  fi
  sleep 1
done

if (( pixels <= 10000 )); then
  cat "$workdir/server.log"
  cat "$workdir/servo.log"
  echo "FAIL: expected JavaScript-colored pixels missing ($pixels)" >&2
  exit 1
fi
if ! grep -q 'GET /probe.html HTTP/1.1" 200' "$workdir/server.log"; then
  cat "$workdir/server.log"
  echo 'FAIL: local page was not requested' >&2
  exit 1
fi
echo "PASS: ServoShell requested local page and rendered $pixels JavaScript-colored pixels"
