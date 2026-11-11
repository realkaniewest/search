# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Run disposable ServoShell storage checks on one local origin."""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlencode, urlsplit


ROOT = Path(__file__).resolve().parent
PASS_COLOR = "14B8A6"
FAIL_COLOR = "DC2626"


def close_color(actual, reference, tolerance):
    try:
        a = bytes.fromhex(actual)
        b = bytes.fromhex(reference)
    except ValueError:
        return False
    return len(a) == len(b) == 3 and all(abs(x - y) <= tolerance for x, y in zip(a, b))


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def log_message(self, _format, *_args):
        pass

    def do_GET(self):
        if urlsplit(self.path).path != "/probe.html":
            self.send_error(404)
            return
        with self.server.probe_gets_lock:
            self.server.probe_gets += 1
        super().do_GET()

    def do_HEAD(self):
        if urlsplit(self.path).path != "/probe.html":
            self.send_error(404)
            return
        super().do_HEAD()


def redact(text):
    text = re.sub(r"(https?://127\.0\.0\.1:\d+/probe\.html)\?[^\s\"']+", r"\1?<redacted>", text)
    return re.sub(r"\b[ABP]_test\b", "<test-marker>", text)


def run_bounded(argv, env=None):
    process = subprocess.Popen(
        argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        start_new_session=True, env=env,
    )
    try:
        stdout, stderr = process.communicate(timeout=45)
    except subprocess.TimeoutExpired as error:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
        raise RuntimeError(f"Timed out: {argv[0]}") from error
    if process.returncode != 0:
        raise RuntimeError(f"Exit {process.returncode}: {argv[0]}\n{redact(stderr[-1200:])}")
    return stdout, stderr


def sample_png(sample_binary, png):
    stdout, _ = run_bounded([str(sample_binary), str(png)])
    colors = stdout.strip().split()
    if len(colors) != 2:
        raise RuntimeError(f"Unexpected PNG sample result: {colors!r}")
    return colors


def inspect_storage(profile):
    files = []
    if profile.exists():
        for path in profile.rglob("*"):
            if path.is_file():
                with path.open("rb") as source:
                    sample = source.read(2 * 1024 * 1024)
                files.append(
                    {
                        "path": str(path.relative_to(profile)),
                        "bytes": path.stat().st_size,
                        "marker_in_first_2MiB": b"P_test" in sample,
                    }
                )
    return sorted(files, key=lambda item: item["path"])


def all_probe_criteria_pass(verdict):
    return all(
        verdict[name]
        for name in (
            "A_B_cookie_checks_pass",
            "A_B_localStorage_checks_pass",
            "temporary_storage_meets_probe_criterion",
        )
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("servoshell", type=Path, help="verified ServoShell v0.8.0 executable")
    options = parser.parse_args()
    binary = options.servoshell.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"servoshell is not executable: {binary}")

    with tempfile.TemporaryDirectory(prefix="search-servo-profiles-") as directory:
        work = Path(directory)
        sample_binary = work / "sample-png"
        build_env = os.environ.copy()
        build_env["CLANG_MODULE_CACHE_PATH"] = str(work / "clang-cache")
        build_env["SWIFT_MODULECACHE_PATH"] = str(work / "swift-cache")
        run_bounded(["swiftc", str(ROOT / "sample.swift"), "-o", str(sample_binary)], env=build_env)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.probe_gets = 0
        server.probe_gets_lock = threading.Lock()
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        origin = f"http://127.0.0.1:{server.server_port}"
        results = []
        calibration = None

        def diagnostics(name, png, get_count, stderr):
            folder = Path(tempfile.mkdtemp(prefix="search-servo-profile-diagnostic-"))
            saved_png = folder / f"{name}.png"
            if png.is_file():
                shutil.copy2(png, saved_png)
            details = {
                "case": name,
                "url_without_query": f"{origin}/probe.html",
                "http_get_count": get_count,
                "png_path": str(saved_png) if saved_png.is_file() else None,
                "png_bytes": saved_png.stat().st_size if saved_png.is_file() else None,
                "servo_stderr_tail": redact(stderr[-1200:]).splitlines()[-12:],
            }
            print(json.dumps({"diagnostic": details}, ensure_ascii=False), flush=True)

        def check(name, profile_name, mode, value=None, expected=None, temporary=False):
            if mode == "control":
                url = f"{origin}/probe.html"
            else:
                params = {"mode": mode}
                if value is not None:
                    params["value"] = value
                if expected is not None:
                    params["expected"] = expected
                url = f"{origin}/probe.html?{urlencode(params)}"
            png = work / f"{name}.png"
            profile = work / profile_name
            argv = [
                str(binary), "--headless", "--exit", f"--output={png}",
                f"--config-dir={profile}",
            ]
            if temporary:
                argv.append("--temporary-storage")
            argv.append(url)
            with server.probe_gets_lock:
                before_gets = server.probe_gets
            _, servo_stderr = run_bounded(argv)
            with server.probe_gets_lock:
                get_count = server.probe_gets - before_gets
            if not png.is_file():
                diagnostics(name, png, get_count, servo_stderr)
                raise RuntimeError(f"ServoShell did not create {name}.png")
            colors = sample_png(sample_binary, png)
            if mode in {"control", "control-red"}:
                reference = PASS_COLOR if mode == "control" else FAIL_COLOR
                if not all(close_color(color, reference, 70) for color in colors):
                    diagnostics(name, png, get_count, servo_stderr)
                    raise RuntimeError(f"Control did not paint a plausible {mode} color: {colors}")
                print(json.dumps({"case": name, "calibration_pixels": colors}, ensure_ascii=False), flush=True)
                return colors

            if calibration is None:
                raise RuntimeError("Controls must calibrate colors before storage checks")
            states = []
            for index, color in enumerate(colors):
                if close_color(color, calibration["green"][index], 10):
                    states.append("pass")
                elif close_color(color, calibration["red"][index], 10):
                    states.append("fail")
                else:
                    diagnostics(name, png, get_count, servo_stderr)
                    raise RuntimeError(f"Unknown probe color in {name}: {colors}")
            row = {
                "case": name,
                "cookie": states[0],
                "localStorage": states[1],
                "pixels": colors,
            }
            results.append(row)
            print(json.dumps(row, ensure_ascii=False), flush=True)

        try:
            green = check("control_green", "control", "control")
            red = check("control_red", "control", "control-red")
            if any(close_color(green[i], red[i], 80) for i in range(2)):
                raise RuntimeError(f"Control colors are not distinct: {green}, {red}")
            calibration = {"green": green, "red": red}
            check("A_set", "A", "set", value="A_test")
            check("B_empty", "B", "read", expected="EMPTY")
            check("B_set", "B", "set", value="B_test")
            check("A_reopen", "A", "read", expected="A_test")
            check("B_reopen", "B", "read", expected="B_test")
            check("P_empty", "P", "read", expected="EMPTY", temporary=True)
            check("P_set", "P", "set", value="P_test", temporary=True)
            check("A_after_P", "A", "read", expected="A_test")
            check("B_after_P", "B", "read", expected="B_test")
            residue = inspect_storage(work / "P")
            print(json.dumps({"P_files_after_main_exit": residue}, ensure_ascii=False), flush=True)
            check("P_reopen_empty", "P", "read", expected="EMPTY", temporary=True)
            check("P_reopen_P", "P", "read", expected="P_test", temporary=True)
            print(json.dumps({"summary": results}, ensure_ascii=False), flush=True)
            by_case = {row["case"]: row for row in results}
            persistent_cases = (
                "A_set", "B_empty", "B_set", "A_reopen", "B_reopen",
                "A_after_P", "B_after_P",
            )
            verdict = {
                "A_B_cookie_checks_pass": all(by_case[name]["cookie"] == "pass" for name in persistent_cases),
                "A_B_localStorage_checks_pass": all(by_case[name]["localStorage"] == "pass" for name in persistent_cases),
                "P_cookie_empty_after_reopen": by_case["P_reopen_empty"]["cookie"] == "pass",
                "P_localStorage_empty_after_reopen": by_case["P_reopen_empty"]["localStorage"] == "pass",
                "P_marker_detected_in_first_2MiB_of_a_file": any(
                    item["marker_in_first_2MiB"] for item in residue
                ),
            }
            verdict["temporary_storage_meets_probe_criterion"] = (
                by_case["P_empty"]["cookie"] == "pass"
                and by_case["P_empty"]["localStorage"] == "pass"
                and by_case["P_set"]["cookie"] == "pass"
                and by_case["P_set"]["localStorage"] == "pass"
                and verdict["P_cookie_empty_after_reopen"]
                and verdict["P_localStorage_empty_after_reopen"]
                and by_case["P_reopen_P"]["cookie"] == "fail"
                and by_case["P_reopen_P"]["localStorage"] == "fail"
                and not verdict["P_marker_detected_in_first_2MiB_of_a_file"]
            )
            verdict["all_probe_criteria_pass"] = all_probe_criteria_pass(verdict)
            print(json.dumps({"verdict": verdict}, ensure_ascii=False), flush=True)
            return 0 if verdict["all_probe_criteria_pass"] else 2
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
