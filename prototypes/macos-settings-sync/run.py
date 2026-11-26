"""Check coordinated snapshot writes from independent local processes."""

import argparse
import json
import os
from pathlib import Path
from subprocess import Popen, PIPE, run
from tempfile import TemporaryDirectory


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--bookmark-reader", type=Path, help="Path to the Search sync_bookmarks_probe example")
args = parser.parse_args()

with TemporaryDirectory(prefix="search-sync-probe-") as temporary:
    work = Path(temporary)
    binary = work / "probe"
    snapshot = work / "snapshot.json"
    run(["swiftc", str(Path(__file__).with_name("probe.swift")), "-o", str(binary)],
        check=True, timeout=60)

    first_revision = run([binary, "seed", snapshot], check=True, text=True,
                         stdout=PIPE, timeout=10).stdout.strip()

    shared = work / "first-connection.json"
    seeds = [Popen([binary, "seed", shared], stdout=PIPE, text=True) for _ in range(2)]
    for seed in seeds:
        seed.communicate(timeout=10)
    assert sorted(seed.returncode for seed in seeds) == [0, 1]
    assert json.loads(shared.read_bytes())["version"] == 1

    initialized = snapshot.read_bytes()
    existing = run([binary, "seed", snapshot], stdout=PIPE, text=True, timeout=10)
    assert existing.returncode == 1 and snapshot.read_bytes() == initialized
    assert not list(work.glob(".search-sync-*"))

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

    base = work / "base.json"
    pending = work / "pending.json"
    base.write_bytes(snapshot.read_bytes())
    pending.write_bytes(base.read_bytes())
    # The offline device edits only its local snapshot, then the other device
    # makes an unrelated update in the shared folder.
    run([binary, "palette", pending, combined["revision"], "system"], check=True, timeout=10)
    pending_revision = json.loads(pending.read_bytes())["revision"]
    run([binary, "page-zoom", pending, pending_revision, "150"], check=True, timeout=10)
    third_address = "https://example.test/third"
    run([*bookmark, combined["revision"], third_address, "Third"], check=True, timeout=10)
    reconcile = [binary, "reconcile", snapshot, base, pending]
    run(reconcile, check=True, timeout=10)
    saved = snapshot.read_bytes()
    merged = json.loads(saved)
    assert merged["palette"] == "system"
    assert merged["pageZoom"] == "150"
    assert merged["bookmarks"] == {**combined["bookmarks"], third_address: "Third"}
    exported = run([binary, "export-bookmarks", snapshot], check=True, stdout=PIPE, timeout=10).stdout
    assert json.loads(exported) == [
        {"url": url, "title": title} for url, title in sorted(merged["bookmarks"].items())
    ]
    if args.bookmark_reader:
        reader = args.bookmark_reader.resolve()
        run([reader], input=exported, check=True, timeout=10)
        duplicate = json.loads(exported)
        duplicate.append(duplicate[0])
        rejected = run([reader], input=json.dumps(duplicate).encode(), stderr=PIPE, timeout=10)
        assert rejected.returncode != 0

    run(reconcile, check=True, timeout=10)
    assert snapshot.read_bytes() == saved

    pending_before = pending.read_bytes()
    run([binary, "palette", snapshot, merged["revision"], "graphite"], check=True, timeout=10)
    divergent = snapshot.read_bytes()
    conflict = run(reconcile, stdout=PIPE, text=True, timeout=10)
    assert (conflict.returncode, conflict.stdout.strip()) == (3, "conflict: palette")
    assert (snapshot.read_bytes(), pending.read_bytes()) == (divergent, pending_before)
    combined = json.loads(divergent)
    base.write_bytes(divergent)
    without_first = {**combined, "bookmarks": {**combined["bookmarks"]}}
    del without_first["bookmarks"]["https://example.test/"]
    pending.write_bytes(divergent)
    profile_bookmarks = [
        {"url": url, "title": title} for url, title in without_first["bookmarks"].items()
    ]
    if args.bookmark_reader:
        run([reader], input=json.dumps(profile_bookmarks).encode(), check=True, timeout=10)
    importer = [binary, "import-bookmarks", pending, combined["revision"]]
    invalid_profile = json.dumps(profile_bookmarks + [profile_bookmarks[0]]).encode()
    rejected = run(importer, input=invalid_profile, stdout=PIPE, timeout=10)
    assert rejected.returncode == 1 and pending.read_bytes() == divergent
    profile_bytes = json.dumps(profile_bookmarks).encode()
    run(importer, input=profile_bytes, check=True, timeout=10)
    after_import = pending.read_bytes()
    stale = run(importer, input=json.dumps([]).encode(), stdout=PIPE, timeout=10)
    assert stale.returncode == 3 and pending.read_bytes() == after_import
    run(reconcile, check=True, timeout=10)
    removed = snapshot.read_bytes()
    run(reconcile, check=True, timeout=10)
    assert snapshot.read_bytes() == removed
    combined = json.loads(removed)
    assert combined["bookmarks"] == without_first["bookmarks"]

    bad = b'{"version":1'
    snapshot.write_bytes(bad)
    rejected = run([binary, "palette", snapshot, combined["revision"], "graphite"],
                   stdout=PIPE, text=True, timeout=10)
    assert rejected.returncode == 1, rejected.stdout
    assert snapshot.read_bytes() == bad
    legacy = work / "legacy.json"
    legacy.write_text(json.dumps({key: value for key, value in combined.items() if key != "pageZoom"}))
    run([binary, "page-zoom", legacy, combined["revision"], "125"], check=True, timeout=10)
    assert json.loads(legacy.read_bytes())["pageZoom"] == "125"

    zoom_base = work / "zoom-base.json"
    zoom_pending = work / "zoom-pending.json"
    zoom_base.write_bytes(legacy.read_bytes())
    zoom_pending.write_bytes(legacy.read_bytes())
    legacy_revision = json.loads(legacy.read_bytes())["revision"]
    run([binary, "page-zoom", legacy, legacy_revision, "110"], check=True, timeout=10)
    run([binary, "page-zoom", zoom_pending, legacy_revision, "200"], check=True, timeout=10)
    shared_before = legacy.read_bytes()
    conflict = run([binary, "reconcile", legacy, zoom_base, zoom_pending],
                   stdout=PIPE, text=True, timeout=10)
    assert (conflict.returncode, conflict.stdout.strip()) == (3, "conflict: pageZoom")
    assert legacy.read_bytes() == shared_before

    invalid_documents = (
        {**combined, "pageZoom": "137"},
        {**combined, "pageZoom": None},
        {**combined, "version": 2},
        {**combined, "privateHistory": ["https://example.test/private"]},
        {**combined, "bookmarks": {"https://user:password@example.test/": "Forbidden"}},
        {**combined, "bookmarks": {"https://127.0.0.1/": "Loopback"}},
        {**combined, "bookmarks": {"https://example.test:8443/": "Other port"}},
        {**combined, "bookmarks": {"https://printer.local/": "Local host"}},
        {**combined, "bookmarks": {"https://example.test/": "Two\nlines"}},
    )
    for document in invalid_documents:
        snapshot.write_text(json.dumps(document), encoding="utf-8")
        original = snapshot.read_bytes()
        rejected = run(reconcile, stdout=PIPE, text=True, timeout=10)
        assert rejected.returncode == 1, rejected.stdout
        assert snapshot.read_bytes() == original

    saved_pending = pending.read_bytes()
    pending.write_bytes(b"{}")
    snapshot.write_bytes(removed)
    rejected = run(reconcile, stdout=PIPE, text=True, timeout=10)
    assert rejected.returncode == 1, rejected.stdout
    assert snapshot.read_bytes() == removed
    pending.write_bytes(saved_pending)

    target = work / "outside-snapshot.json"
    target.write_bytes(removed)
    snapshot.unlink()
    snapshot.symlink_to(target)
    rejected = run(reconcile, stdout=PIPE, text=True, timeout=10)
    assert rejected.returncode == 1 and snapshot.is_symlink(), rejected.stdout
    assert target.read_bytes() == removed

    snapshot.unlink()
    os.mkfifo(snapshot)
    rejected = run(reconcile, stdout=PIPE, text=True, timeout=10)
    assert rejected.returncode == 1 and snapshot.is_fifo(), rejected.stdout

    print("PASS: competing writers detected a conflict, offline edits merged, divergent edits conflicted, and invalid files were left untouched")
