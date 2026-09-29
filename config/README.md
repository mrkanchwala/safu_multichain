# Pool network files

ONE switch per pool: `SAFU_POOL_NETWORK=testnet|mainnet` picks `pool.<network>.json`, and the whole pool follows
(backend, frontend build, nginx security headers). Separate from the scanner's switch (`SAFU_NETWORK_<PRODUCT>`,
SAFU3.0 `networks.py`): a pool and its scanner may sit on different networks.

- No default: unset or unknown -> the backend refuses to start and the frontend build fails.
- `null` = not deployed / not set yet. The backend refuses to start while any value it uses is null.
- `contracts` + `pool.cap_usdc` are written by `scripts/deploy/deploy_v1.sh`, never by hand.
- Every other value was checked on 2026-09-29 against Circle's docs AND the chain itself (mainnet), or the live
  testnet build (testnet). Stellar `usdc_sac` is derived from issuer + passphrase; a test re-derives it.
