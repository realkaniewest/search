"""Run the bounded first-window startup check without leaving a GUI process behind."""

import subprocess
import sys


def main() -> int:
    command = sys.argv[1:]
    if not command:
        print("Usage: run_search_smoke.py <command> [args...]", file=sys.stderr)
        return 2
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=50, check=False)
    except subprocess.TimeoutExpired as error:
        if error.stdout:
            print(error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else error.stdout, end="")
        if error.stderr:
            print(error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else error.stderr, end="", file=sys.stderr)
        print("FAIL: Search first window smoke timed out", file=sys.stderr)
        return 1
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    if result.returncode != 0 or "PASS: Search chrome and start content loaded" not in result.stdout:
        print("FAIL: Search first window smoke did not pass", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
