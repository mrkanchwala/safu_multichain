#!/usr/bin/env bash
# Set up the pool's DeFindex yield vault so the last staker can always leave
# and the money actually earns.
#
# Usage: scripts/deploy/setup_vault.sh <vault.env>
#   config keys: see scripts/deploy/vault.env.example
#   DRY_RUN=1 simulates the vault creation only and sends nothing.
#
# In order, and it stops at the first failure:
#   1. Check the Blend strategy holds the pool's asset (2nd source after DeFindex's own list).
#   2. Create the vault AND make its first deposit in one transaction, from a separate SEEDER
#      wallet. DeFindex locks part of the first deposit's shares forever; with the seeder they
#      are the seeder's, never the pool's (the 2026-10-01 error 80 on the last withdraw).
#   3. Read back roles and fee.
#   4. Rebalance: the rebalance manager invests everything idle into the Blend strategy
#      (a vault starts 100% idle and every later deposit copies that split: the 0% yield
#      found 2026-10-01).
#   5. Assert idle == 0 and invested > 0. On failure the vault is NOT handed to the pool.
#   6. If POOL is set: fund the pool's rounding reserve and read it back.
# Setting the vault on the pool stays the two-role setup step (GovChange::Vault), after this.
#
# BLND rewards: the strategy's harvest is run by DeFindex's own keeper, shared by every vault on
# that strategy (get_keeper, checked 2026-10-19). Blend's interest needs no harvest. Nothing here
# or in the pool calls it (SAFU runs no keeper).
set -euo pipefail

CONFIG=${1:?usage: setup_vault.sh <vault.env>}
# shellcheck disable=SC1090
. "$CONFIG"

need() { for k in "$@"; do [ -n "${!k:-}" ] || { echo "missing $k in $CONFIG" >&2; exit 1; }; done; }
need NETWORK ASSET_TOKEN DEFINDEX_FACTORY BLEND_STRATEGY STRATEGY_NAME SOROSWAP_ROUTER \
     ADMIN GUARDIAN ADMIN_SOURCE SEEDER SEED_AMOUNT VAULT_NAME VAULT_SYMBOL

PASSPHRASE=$(stellar network ls --long 2>/dev/null | awk -v n="$NETWORK" \
  '$1=="Name:"{m=($2==n)} m && /^Network passphrase:/{sub(/^Network passphrase: /,""); print; exit}')
[ -n "$PASSPHRASE" ] || { echo "network $NETWORK is not configured in stellar-cli" >&2; exit 1; }
IS_MAINNET=0
[ "$PASSPHRASE" != "Public Global Stellar Network ; September 2015" ] || IS_MAINNET=1
if [ "$IS_MAINNET" = 1 ] && [ "${DRY_RUN:-}" != 1 ] && [ "${CONFIRM_MAINNET:-}" != "yes" ]; then
  echo "mainnet run refused: set CONFIRM_MAINNET=yes in the environment for this run" >&2
  exit 1
fi
# DeFindex locks 1,000 units of the first deposit; the seed must clear it with room to spare.
[ "$SEED_AMOUNT" -ge 10000 ] || { echo "SEED_AMOUNT must be at least 10000 units" >&2; exit 1; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
N=(--network "$NETWORK")
view() { local id=$1; shift; stellar contract invoke --id "$id" "${N[@]}" --source "$ADMIN_SOURCE" --send=no -- "$@" 2>/dev/null | tail -1 | tr -d '"'; }
fail=0
check() { if [ "$2" = "$3" ]; then echo "  ok   $1"; else echo "  FAIL $1: got $2, want $3"; fail=1; fi; }
stop() { echo "STOPPED: $1" >&2; exit 1; }

# --- 1. Strategy asset ------------------------------------------------------------------------
echo "checking the Blend strategy's asset"
check "strategy asset" "$(view "$BLEND_STRATEGY" asset)" "$ASSET_TOKEN"
[ $fail = 0 ] || stop "the strategy does not hold the pool's asset"

# --- 2. Create + seed in one transaction -------------------------------------------------------
# Roles (DeFindex vault/src/access.rs): 0 emergency, 1 fee receiver, 2 manager, 3 rebalance.
ROLES="{\"0\":\"$GUARDIAN\",\"1\":\"$ADMIN\",\"2\":\"$ADMIN\",\"3\":\"$ADMIN\"}"
ASSETS="[{\"address\":\"$ASSET_TOKEN\",\"strategies\":[{\"address\":\"$BLEND_STRATEGY\",\"name\":\"$STRATEGY_NAME\",\"paused\":false}]}]"
NAMES="{\"name\":\"$VAULT_NAME\",\"symbol\":\"$VAULT_SYMBOL\"}"
SEEDER_ADDR=$(stellar keys address "$SEEDER")
create() {
  stellar contract invoke --id "$DEFINDEX_FACTORY" "${N[@]}" --source "$SEEDER" "$@" -- \
    create_defindex_vault_deposit --caller "$SEEDER_ADDR" --roles "$ROLES" --vault_fee 0 \
    --assets "$ASSETS" --soroswap_router "$SOROSWAP_ROUTER" --name_symbol "$NAMES" \
    --upgradable false --amounts "[\"$SEED_AMOUNT\"]" | tail -1 | tr -d '"'
}
if [ "${DRY_RUN:-}" = 1 ]; then
  echo "dry run: simulating create + seed ($SEED_AMOUNT units from $SEEDER_ADDR)"
  SIM=$(create --send=no) || stop "simulation failed (seeder funded, with a trustline to the asset?)"
  echo "  would create: $SIM"
  exit 0
fi
echo "creating the vault and seeding it with $SEED_AMOUNT units from $SEEDER_ADDR"
VAULT=$(create) || stop "vault creation failed; nothing was handed to the pool"
case $VAULT in C*) ;; *) stop "vault creation returned '$VAULT'";; esac
echo "  vault $VAULT"

# --- 3. Read back -----------------------------------------------------------------------------
echo "read-back:"
check "emergency manager" "$(view "$VAULT" get_emergency_manager)" "$GUARDIAN"
check "fee receiver"      "$(view "$VAULT" get_fee_receiver)"      "$ADMIN"
check "manager"           "$(view "$VAULT" get_manager)"           "$ADMIN"
check "rebalance manager" "$(view "$VAULT" get_rebalance_manager)" "$ADMIN"
check "vault fee"         "$(view "$VAULT" get_fees | tr -d '[] ' | cut -d, -f1)" "0"
check "seeder holds shares" "$( [ "$(view "$VAULT" balance --id "$SEEDER_ADDR")" -gt 0 ] && echo yes || echo no)" "yes"
[ $fail = 0 ] || stop "read-back failed; vault $VAULT must not be set on the pool"

# --- 4 + 5. Rebalance into Blend, then assert nothing is idle ------------------------------------
funds() { view "$VAULT" fetch_total_managed_funds | python3 -c '
import json, sys
a = json.loads(sys.stdin.read())[0]
print(a["idle_amount"], a["invested_amount"])'; }
read -r IDLE INVESTED <<< "$(funds)"
echo "before rebalance: idle $IDLE, invested $INVESTED"
if [ "$IDLE" -gt 0 ]; then
  stellar contract invoke --id "$VAULT" "${N[@]}" --source "$ADMIN_SOURCE" -- rebalance \
    --caller "$ADMIN" --instructions "[{\"Invest\":[\"$BLEND_STRATEGY\",\"$IDLE\"]}]" > /dev/null
fi
read -r IDLE INVESTED <<< "$(funds)"
echo "after rebalance:  idle $IDLE, invested $INVESTED"
check "idle is 0"        "$IDLE" "0"
check "invested above 0" "$( [ "$INVESTED" -gt 0 ] && echo yes || echo no)" "yes"
[ $fail = 0 ] || stop "vault $VAULT is not fully invested; do not set it on the pool"

# --- 6. Rounding reserve (optional, needs the pool) ---------------------------------------------
if [ -n "${POOL:-}" ]; then
  need RESERVE_FUNDER RESERVE_AMOUNT
  echo "funding the pool's rounding reserve with $RESERVE_AMOUNT units"
  BEFORE=$(view "$POOL" get_rounding_reserve)
  stellar contract invoke --id "$POOL" "${N[@]}" --source "$RESERVE_FUNDER" -- fund_rounding_reserve \
    --from "$(stellar keys address "$RESERVE_FUNDER")" --amount "$RESERVE_AMOUNT" > /dev/null
  check "rounding reserve" "$(view "$POOL" get_rounding_reserve)" "$((BEFORE + RESERVE_AMOUNT))"
fi

# --- Record -----------------------------------------------------------------------------------
mkdir -p "$ROOT/deploy/out"
OUT=$ROOT/deploy/out/vault-$NETWORK-$(date -u +%Y%m%dT%H%M%SZ).json
cat > "$OUT" <<EOF
{
  "network": "$NETWORK",
  "created_at": "$(date -u +%FT%TZ)",
  "vault": "$VAULT",
  "asset": "$ASSET_TOKEN",
  "strategy": "$BLEND_STRATEGY",
  "factory": "$DEFINDEX_FACTORY",
  "seeder": "$SEEDER_ADDR",
  "seed_amount": "$SEED_AMOUNT",
  "idle_after_rebalance": "$IDLE",
  "invested_after_rebalance": "$INVESTED",
  "pool": "${POOL:-}",
  "rounding_reserve_funded": "${POOL:+$RESERVE_AMOUNT}",
  "checks_ok": $([ $fail = 0 ] && echo true || echo false)
}
EOF
echo "written: $OUT"
[ $fail = 0 ] && echo "VAULT READY: set it on the pool with GovChange::Vault($VAULT) (two roles)."
exit $fail
