#!/usr/bin/env bash
# Fails if a rule number is typed outside its one source (Stellar contracts).
#   Rule numbers, setting defaults -> contracts/protection-pool/src/types.rs
#   Setting bounds                 -> contracts/protection-pool/src/settings.rs
#   Points scale                   -> contracts/protection-pool/src/points.rs
#   Error codes (public ABI)       -> contracts/protection-pool/src/error.rs
#   Circle CCTP layout and domains -> contracts/cctp-common (facts about an outside protocol)
# Tests may use literal fixtures. A line can opt out with the marker `hardcode-ok: <reason>`.
set -euo pipefail
cd "$(dirname "$0")/../contracts"

SOURCES=(protection-pool/src covered-registry/src cctp-adapter/src safu-account/src)
ALLOWED='protection-pool/src/(types|settings|points|error)\.rs'

hits=$(grep -rnE '(^|[^A-Za-z0-9_."#])[0-9][0-9_]{2,}([^0-9_]|$)' "${SOURCES[@]}" --include='*.rs' \
  | grep -v '/test' | grep -vE '^[^:]+:[0-9]+:\s*//' | grep -vE "$ALLOWED" | grep -v 'hardcode-ok:' || true)
if [ -n "$hits" ]; then
  echo "Number typed outside its source:"; echo "$hits"; exit 1
fi
echo "check_no_hardcodes: clean"
