#!/usr/bin/env bash
# Build the fast-clock WASMs for the testnet end-to-end run: the production contracts with the
# duration constants in fast_constants.txt shortened to minutes.
#
# Usage: scripts/e2e/build_fast.sh
#
# contracts/ is never edited. The source is copied to .cache/fast-build/ (gitignored), the
# allowlisted lines are replaced there, and both trees are built. The build fails unless:
#   - contracts/ is clean against HEAD (set ALLOW_DIRTY=1 to override for a dry run),
#   - every allowlisted production line exists exactly once,
#   - the two trees differ in exactly the allowlisted lines and nothing else (checked by a
#     separate tree walk, not by trusting the patch step),
#   - covered_registry, cctp_adapter and safu_account come out byte-identical to production,
#     and protection_pool comes out different.
# Output: deploy/out/fast-build-<utc>/ with wasm/, constants.diff and build.json. Deploy with
#   WASM_DIR=<that dir>/wasm scripts/deploy/deploy_v1.sh <config.env>
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ALLOW=$ROOT/scripts/e2e/fast_constants.txt
FAST=$ROOT/.cache/fast-build
OUT=$ROOT/deploy/out/fast-build-$(date -u +%Y%m%dT%H%M%SZ)
PACKAGES=(protection-pool covered-registry cctp-adapter safu-account)
REL=target/wasm32v1-none/release
export PYTHONPATH="$ROOT/scripts/e2e${PYTHONPATH:+:$PYTHONPATH}"

if ! git -C "$ROOT" diff --quiet HEAD -- contracts || [ -n "$(git -C "$ROOT" ls-files --others --exclude-standard -- contracts)" ]; then
  if [ "${ALLOW_DIRTY:-}" != "1" ]; then
    echo "contracts/ has uncommitted changes: fast-clock must come from a commit (ALLOW_DIRTY=1 for a dry run)" >&2
    exit 1
  fi
  DIRTY=-dirty
else
  DIRTY=
fi

# --- Copy and patch ------------------------------------------------------------
# target/ is kept between runs so rebuilds are incremental; sources are replaced in full.
mkdir -p "$FAST/contracts"
rsync -a --delete --exclude target "$ROOT/contracts/" "$FAST/contracts/"

python3 - "$ALLOW" "$FAST/contracts" <<'EOF'
import sys
from pathlib import Path
from e2e_allowlist import load
for path, prod, fast in load(sys.argv[1]):
    f = Path(sys.argv[2]) / path
    lines = f.read_text().split("\n")
    hits = [i for i, l in enumerate(lines) if l.strip() == prod]
    if len(hits) != 1:
        sys.exit(f"{path}: expected exactly 1 line '{prod}', found {len(hits)}")
    i = hits[0]
    lines[i] = lines[i][: len(lines[i]) - len(lines[i].lstrip())] + fast
    f.write_text("\n".join(lines))
EOF

# --- Diff guard ----------------------------------------------------------------
mkdir -p "$OUT"
python3 - "$ALLOW" "$ROOT/contracts" "$FAST/contracts" "$OUT/constants.diff" <<'EOF'
import difflib, sys
from pathlib import Path
from e2e_allowlist import load
allow_file, prod_root, fast_root, diff_out = sys.argv[1:]
prod_root, fast_root = Path(prod_root), Path(fast_root)
allowed = {(p, a, b) for p, a, b in load(allow_file)}

def files(root):
    return {p.relative_to(root).as_posix() for p in root.rglob("*")
            if p.is_file() and "target" not in p.relative_to(root).parts}

prod_files, fast_files = files(prod_root), files(fast_root)
if prod_files != fast_files:
    sys.exit(f"file sets differ: {sorted(prod_files ^ fast_files)}")

seen, report = set(), []
for rel in sorted(prod_files):
    a = (prod_root / rel).read_bytes()
    b = (fast_root / rel).read_bytes()
    if a == b:
        continue
    al, bl = a.decode().split("\n"), b.decode().split("\n")
    if len(al) != len(bl):
        sys.exit(f"{rel}: line count changed ({len(al)} -> {len(bl)})")
    for x, y in zip(al, bl):
        if x == y:
            continue
        key = (rel, x.strip(), y.strip())
        if key not in allowed:
            sys.exit(f"{rel}: change not on the allowlist:\n  - {x}\n  + {y}")
        seen.add(key)
    report += difflib.unified_diff(al, bl, f"contracts/{rel}", f"fast/{rel}", lineterm="")
if seen != allowed:
    sys.exit(f"allowlisted changes not applied: {sorted(allowed - seen)}")
Path(diff_out).write_text("\n".join(report) + "\n")
print(f"diff guard ok: {len(seen)} allowlisted lines changed, nothing else")
EOF

# --- Build both trees ----------------------------------------------------------
build() { (cd "$1" && for p in "${PACKAGES[@]}"; do stellar contract build --package "$p" > /dev/null; done); }
echo "building production tree"; build "$ROOT/contracts"
echo "building fast tree";       build "$FAST/contracts"

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
mkdir -p "$OUT/wasm"
fail=0
hashes=()
for w in protection_pool covered_registry cctp_adapter safu_account; do
  p=$(sha "$ROOT/contracts/$REL/$w.wasm"); f=$(sha "$FAST/contracts/$REL/$w.wasm")
  cp "$FAST/contracts/$REL/$w.wasm" "$OUT/wasm/"
  if [ "$w" = protection_pool ]; then
    [ "$p" != "$f" ] || { echo "FAIL $w: fast build is identical to production" >&2; fail=1; }
  else
    [ "$p" = "$f" ] || { echo "FAIL $w: fast build differs from production" >&2; fail=1; }
  fi
  hashes+=("$w" "$p" "$f")
done

python3 - "$OUT/build.json" "$(git -C "$ROOT" rev-parse HEAD)$DIRTY" "$(sha "$ALLOW")" "$fail" "${hashes[@]}" <<'EOF'
import datetime, json, sys
out, commit, allow_sha, fail, *h = sys.argv[1:]
json.dump({
    "built_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "git_commit": commit,
    "allowlist_sha256": allow_sha,
    "wasm_sha256": {h[i]: {"prod": h[i + 1], "fast": h[i + 2]} for i in range(0, len(h), 3)},
    "guard_ok": fail == "0",
}, open(out, "w"), indent=2)
EOF
echo "written: $OUT"
[ $fail = 0 ] && echo "deploy with: WASM_DIR=$OUT/wasm scripts/deploy/deploy_v1.sh <config.env>"
exit $fail
