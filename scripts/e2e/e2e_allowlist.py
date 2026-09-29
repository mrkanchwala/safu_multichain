"""Parser for fast_constants.txt, shared by build_fast.sh's patch step and its diff guard."""

from pathlib import Path


def load(path):
    """Return [(file under contracts/, production line, fast line)], all trimmed."""
    entries = []
    for n, raw in enumerate(Path(path).read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [p.strip() for p in line.split(" | ")]
        if len(parts) != 3 or not all(parts) or parts[1] == parts[2]:
            raise SystemExit(f"{path}:{n}: expected '<file> | <prod line> | <fast line>'")
        entries.append(tuple(parts))
    if len(set(entries)) != len(entries):
        raise SystemExit(f"{path}: duplicate entries")
    return entries
