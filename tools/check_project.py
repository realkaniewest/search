"""Check project configuration and local documentation links."""

from pathlib import Path
from urllib.parse import unquote, urlsplit
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
SKIP_DIRS = {".git", ".lattice", "target"}


def project_files(suffix: str):
    for path in ROOT.rglob(f"*{suffix}"):
        if not any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
            yield path


def main() -> int:
    errors: list[str] = []

    for path in project_files(".toml"):
        try:
            tomllib.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
            errors.append(f"{path.relative_to(ROOT)}: invalid TOML: {exc}")

    for path in project_files(".md"):
        content = path.read_text(encoding="utf-8")
        for target in LINK.findall(content):
            if target.startswith(("#", "mailto:")):
                continue
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc:
                continue
            candidate = path.parent / unquote(parsed.path)
            if not candidate.exists():
                errors.append(f"{path.relative_to(ROOT)}: missing link {target}")

    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        return 1
    print("Project TOML and local Markdown links are valid.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
