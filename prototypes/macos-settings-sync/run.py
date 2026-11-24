"""Check coordinated snapshot writes from independent local processes."""

import json
from pathlib import Path
from subprocess import Popen, PIPE, run
from tempfile import TemporaryDirectory


with TemporaryDirectory(prefix="search-sync-probe-") as temporary:
    work = Path(temporary)
    binary = work / "probe"
    snapshot = work / "snapshot.json"
    run(["swiftc", str(Path(__file__).with_name("probe.swift")), "-o", str(binary)],
        check=True, timeout=60)

    first_revision = run([binary, "seed", snapshot], check=True, text=True,
                         stdout=PIPE, timeout=10).stdout.strip()

    contenders = [
        Popen([binary, "palette", snapshot, first_revision, palette], stdout=PIPE, text=True)
        for palette in ("paper", "midnight")
    ]
    try:
        results = [process.communicate(timeout=10)[0] for process in contenders]
    finally:
        for process in contenders:
            if process.poll() is None:
                process.kill()
                process.communicate()
    codes = [process.returncode for process in contenders]
    assert sorted(codes) == [0, 3], (codes, results)
    current = json.loads(snapshot.read_bytes())
    assert current["palette"] == ("paper" if codes[0] == 0 else "midnight")
    assert current["bookmarks"] == {"https://example.test/": "Fixture"}
    assert current["compactTabs"] is True
    assert current["revision"] != first_revision

    bookmark = [binary, "bookmark", snapshot]
    address = "https://example.test/second"
    conflict = run([*bookmark, first_revision, address, "Second"],
                   stdout=PIPE, text=True, timeout=10)
    assert conflict.returncode == 3, conflict.stdout
    run([*bookmark, current["revision"], address, "Second"], check=True, timeout=10)
    combined = json.loads(snapshot.read_bytes())
    assert combined["palette"] == current["palette"]
    assert combined["bookmarks"] == {"https://example.test/": "Fixture", address: "Second"}

    bad = b'{"version":1'
    snapshot.write_bytes(bad)
    rejected = run([binary, "palette", snapshot, combined["revision"], "graphite"],
                   stdout=PIPE, text=True, timeout=10)
    assert rejected.returncode == 1, rejected.stdout
    assert snapshot.read_bytes() == bad
    print("PASS: competing writers detected a conflict, retry preserved both edits, and corrupt data was untouched")
