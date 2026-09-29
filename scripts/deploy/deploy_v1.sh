#!/usr/bin/env bash
# Deploy the v1 Soroban stack: protection-pool, covered-registry, and the two
# CCTP adapters (Stake + Back). safu-account is uploaded only; each adapter
# deploys a user's account from its WASM hash on first deposit.
#
# Usage: scripts/deploy/deploy_v1.sh <config.env>
#   config keys: see scripts/deploy/testnet.env.example
#
# Every constructor argument comes from the config file, never from a default,
# so a mainnet run cannot silently inherit a testnet value. After deploying it
# reads every role back from the chain and fails on any mismatch. The result
# (contract IDs + WASM sha256) is written to deploy/out/<network>-<utc>.json.
# On a clean read-back the ids + pool cap are also written into config/pool.<network>.json (C1).
set -euo pipefail

CONFIG=${1:?usage: deploy_v1.sh <config.env>}
# shellcheck disable=SC1090
. "$CONFIG"

need() { for k in "$@"; do [ -n "${!k:-}" ] || { echo "missing $k in $CONFIG" >&2; exit 1; }; done; }
need NETWORK SOURCE ADMIN CO_SIGNER GUARDIAN ORACLE ORACLE_PUBKEY ASSET_TOKEN POOL_CAP \
     REGISTRY_WRITER CCTP_TRANSMITTER CCTP_MESSENGER

# Mainnet is detected by passphrase, not by name: an alias such as safu-mainnet
# points at the same network.
PASSPHRASE=$(stellar network ls --long 2>/dev/null | awk -v n="$NETWORK" \
  '$1=="Name:"{m=($2==n)} m && /^Network passphrase:/{sub(/^Network passphrase: /,""); print; exit}')
[ -n "$PASSPHRASE" ] || { echo "network $NETWORK is not configured in stellar-cli" >&2; exit 1; }
IS_MAINNET=0
[ "$PASSPHRASE" != "Public Global Stellar Network ; September 2015" ] || IS_MAINNET=1

if [ "$IS_MAINNET" = 1 ] && [ "${CONFIRM_MAINNET:-}" != "yes" ]; then
  echo "mainnet deploy refused: set CONFIRM_MAINNET=yes in the environment for this run" >&2
  exit 1
fi

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# WASM_DIR from the environment deploys prebuilt WASMs (the D-fast build from
# scripts/e2e/build_fast.sh) and skips the build, which would only rebuild contracts/.
if [ -n "${WASM_DIR:-}" ]; then
  [ "$IS_MAINNET" = 0 ] || { echo "WASM_DIR override refused on mainnet" >&2; exit 1; }
  # Only a build whose diff guard passed, with the files unchanged since.
  python3 - "$WASM_DIR" <<'EOF'
import hashlib, json, sys
from pathlib import Path
wasm = Path(sys.argv[1])
rec = wasm.parent / "build.json"
if not rec.is_file():
    sys.exit(f"WASM_DIR: no build.json next to {wasm} (use a build_fast.sh output)")
b = json.loads(rec.read_text())
if b.get("guard_ok") is not True:
    sys.exit(f"WASM_DIR: {rec} says the diff guard failed")
for name, h in b["wasm_sha256"].items():
    if hashlib.sha256((wasm / f"{name}.wasm").read_bytes()).hexdigest() != h["fast"]:
        sys.exit(f"WASM_DIR: {name}.wasm does not match {rec}")
EOF
  SKIP_BUILD=1
else
  WASM_DIR=$ROOT/contracts/target/wasm32v1-none/release
fi
S=(--source "$SOURCE" --network "$NETWORK")

# --- Preflight ---------------------------------------------------------------
# Admin, co-signer, guardian and oracle must be four different addresses
# (the pool enforces it too; failing here is cheaper than a failed deploy).
if [ "$(printf '%s\n' "$ADMIN" "$CO_SIGNER" "$GUARDIAN" "$ORACLE" | sort -u | wc -l | tr -d ' ')" != 4 ]; then
  echo "admin, co_signer, guardian and oracle must all differ" >&2; exit 1
fi
# The pool constructor calls admin.require_auth(), so the deploy must be signed
# by the admin: SOURCE has to be the admin's own identity.
if [ "$(stellar keys address "$SOURCE" 2>/dev/null)" != "$ADMIN" ]; then
  echo "SOURCE ($SOURCE) is not ADMIN: the pool constructor needs the admin's signature" >&2; exit 1
fi
# The oracle's G-address and its raw Ed25519 key must be the same key, or every
# claim fails to verify.
python3 - "$ORACLE" "$ORACLE_PUBKEY" <<'EOF'
import sys
from stellar_sdk import Keypair
addr, raw = sys.argv[1], sys.argv[2]
if Keypair.from_raw_ed25519_public_key(bytes.fromhex(raw)).public_key != addr:
    sys.exit(f"ORACLE_PUBKEY does not derive ORACLE ({addr})")
EOF

if [ "${SKIP_BUILD:-}" != "1" ]; then
  (cd "$ROOT/contracts" && for p in protection-pool covered-registry cctp-adapter safu-account; do
     stellar contract build --package "$p" > /dev/null; done)
fi
for w in protection_pool covered_registry cctp_adapter safu_account; do
  [ -f "$WASM_DIR/$w.wasm" ] || { echo "missing $WASM_DIR/$w.wasm" >&2; exit 1; }
done
sha() { shasum -a 256 "$WASM_DIR/$1.wasm" | cut -d' ' -f1; }

# --- Deploy ------------------------------------------------------------------
echo "deploying protection-pool ($NETWORK)"
POOL=$(stellar contract deploy --wasm "$WASM_DIR/protection_pool.wasm" "${S[@]}" -- \
  --admin "$ADMIN" --oracle "$ORACLE" --oracle_pubkey "$ORACLE_PUBKEY" \
  --co_signer "$CO_SIGNER" --guardian "$GUARDIAN" --xlm_token "$ASSET_TOKEN" \
  --pool_cap "$POOL_CAP" | tail -1)

echo "deploying covered-registry"
REGISTRY=$(stellar contract deploy --wasm "$WASM_DIR/covered_registry.wasm" "${S[@]}" -- \
  --writer "$REGISTRY_WRITER" --pool "$POOL" | tail -1)

echo "uploading safu-account"
ACCOUNT_HASH=$(stellar contract upload --wasm "$WASM_DIR/safu_account.wasm" "${S[@]}" | tail -1)

deploy_adapter() {
  stellar contract deploy --wasm "$WASM_DIR/cctp_adapter.wasm" "${S[@]}" -- \
    --transmitter "$CCTP_TRANSMITTER" --messenger "$CCTP_MESSENGER" --usdc "$ASSET_TOKEN" \
    --pool "$POOL" --account_wasm "$ACCOUNT_HASH" --mode "$1" | tail -1
}
echo "deploying staking adapter"
STAKE_ADAPTER=$(deploy_adapter Stake)
echo "deploying backing adapter"
BACK_ADAPTER=$(deploy_adapter Back)

# --- Read back and check -----------------------------------------------------
view() { local id=$1; shift; stellar contract invoke --id "$id" "${S[@]}" --send=no -- "$@" 2>/dev/null | tail -1 | tr -d '"'; }
fail=0
check() { if [ "$2" = "$3" ]; then echo "  ok   $1"; else echo "  FAIL $1: got $2, want $3"; fail=1; fi; }
echo "read-back:"
check "pool admin"       "$(view "$POOL" get_admin)"     "$ADMIN"
check "pool co_signer"   "$(view "$POOL" get_co_signer)" "$CO_SIGNER"
check "pool guardian"    "$(view "$POOL" get_guardian)"  "$GUARDIAN"
check "pool oracle"      "$(view "$POOL" get_oracle)"    "$ORACLE"
check "registry writer"  "$(view "$REGISTRY" get_writer)" "$REGISTRY_WRITER"
check "registry pool"    "$(view "$REGISTRY" get_pool)"   "$POOL"
check "stake adapter pool" "$(view "$STAKE_ADAPTER" pool)" "$POOL"
check "stake adapter mode" "$(view "$STAKE_ADAPTER" mode)" "Stake"
check "back adapter pool"  "$(view "$BACK_ADAPTER" pool)"  "$POOL"
check "back adapter mode"  "$(view "$BACK_ADAPTER" mode)"  "Back"

# --- Record ------------------------------------------------------------------
mkdir -p "$ROOT/deploy/out"
OUT=$ROOT/deploy/out/$NETWORK-$(date -u +%Y%m%dT%H%M%SZ).json
cat > "$OUT" <<EOF
{
  "network": "$NETWORK",
  "deployed_at": "$(date -u +%FT%TZ)",
  "git_commit": "$(git -C "$ROOT" rev-parse HEAD)$(git -C "$ROOT" diff --quiet -- contracts || echo '-dirty')",
  "wasm_dir": "$WASM_DIR",
  "pool": "$POOL",
  "registry": "$REGISTRY",
  "stake_adapter": "$STAKE_ADAPTER",
  "back_adapter": "$BACK_ADAPTER",
  "account_wasm_hash": "$ACCOUNT_HASH",
  "roles": {"admin": "$ADMIN", "co_signer": "$CO_SIGNER", "guardian": "$GUARDIAN",
            "oracle": "$ORACLE", "registry_writer": "$REGISTRY_WRITER"},
  "wasm_sha256": {"protection_pool": "$(sha protection_pool)", "covered_registry": "$(sha covered_registry)",
                  "cctp_adapter": "$(sha cctp_adapter)", "safu_account": "$(sha safu_account)"},
  "readback_ok": $([ $fail = 0 ] && echo true || echo false)
}
EOF
echo "written: $OUT"

# --- Pool network file (C1, 2026-09-29) ---------------------------------------------------------
# The ONE place the backend, frontend build and nginx read contract ids from. Written only after a
# clean read-back, and only if that file's passphrase is this network's (never cross-write).
if [ $fail = 0 ]; then
  POOL_FILE=$ROOT/config/pool.$NETWORK.json
  python3 - "$POOL_FILE" "$OUT" "$PASSPHRASE" "$POOL_CAP" <<'PY'
import json, sys
pool_file, out, passphrase, cap = sys.argv[1:]
cfg, dep = json.load(open(pool_file)), json.load(open(out))
if cfg["stellar"]["passphrase"] != passphrase:
    sys.exit(f"{pool_file} is for {cfg['stellar']['passphrase']!r}, this deploy is {passphrase!r}; not written")
cfg["contracts"] = {k: dep[k] for k in ("pool", "registry", "stake_adapter", "back_adapter")}
cfg["contracts"]["deploy_record"] = out[out.index("deploy/out/"):]
cfg["pool"]["cap_usdc"] = int(cap) // 10**7
json.dump(cfg, open(pool_file, "w"), indent=1)
open(pool_file, "a").write("\n")
print(f"pool file updated: {pool_file} (commit it; rebuild the frontend; restart the backend)")
PY
  [ $? = 0 ] || { echo "POOL FILE NOT UPDATED: backend and frontend still point at the old contracts" >&2; fail=1; }
fi
exit $fail
