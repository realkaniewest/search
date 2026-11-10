#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
set -euo pipefail

binary=${1:?pass the built Linux probe binary}
workdir=$(mktemp -d)
app_pid=
cleanup() {
  if [[ -n "$app_pid" ]]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -rf "$workdir"
}
trap cleanup EXIT

SEARCH_LINUX_INTERACTIVE=1 "$binary" >"$workdir/app.log" 2>&1 &
app_pid=$!

ready=false
for _ in {1..120}; do
  if grep -q 'READY: waiting for screenshot and input' "$workdir/app.log"; then
    ready=true
    break
  fi
  if ! kill -0 "$app_pid" 2>/dev/null; then
    cat "$workdir/app.log"
    echo 'FAIL: app exited before ready marker' >&2
    exit 1
  fi
  sleep 0.5
done
if [[ "$ready" != true ]]; then
  cat "$workdir/app.log"
  echo 'FAIL: no ready marker' >&2
  exit 1
fi

window_id=$(xdotool search --onlyvisible --name '^Search Linux Wry runtime probe$' | head -n 1)
if [[ -z "$window_id" ]]; then
  echo 'FAIL: Tao window not found' >&2
  exit 1
fi

colored_pixels=0
for _ in {1..20}; do
  if import -window "$window_id" "$workdir/window.png" 2>"$workdir/import.err"; then
    colored_pixels=$(convert "$workdir/window.png" -format %c histogram:info:- | awk '/#14B8A6/ { sum += $1 } END { print sum + 0 }')
    if (( colored_pixels > 10000 )); then
      break
    fi
  fi
  sleep 0.5
done
if (( colored_pixels <= 10000 )); then
  cat "$workdir/app.log"
  cat "$workdir/import.err" >&2
  echo "FAIL: screenshot contains only $colored_pixels expected teal pixels" >&2
  exit 1
fi
dimensions=$(identify -format '%wx%h' "$workdir/window.png")
echo "PIXELS: captured $dimensions window with $colored_pixels expected teal pixels"

xdotool mousemove --window "$window_id" 100 300
xdotool click 1
xdotool type --clearmodifiers --delay 10 'SEARCH_INPUT_OK'
if ! wait "$app_pid"; then
  cat "$workdir/app.log"
  echo 'FAIL: app exited before input marker' >&2
  exit 1
fi
app_pid=
cat "$workdir/app.log"
grep -q 'PASS: X11 click and keyboard input reached the page' "$workdir/app.log"
