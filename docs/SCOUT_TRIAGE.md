# Scout results: review of every medium finding

Run `scout-2` (CoinFabrik Scout, `.github/workflows/scout.yml`), 2026-10-19, on the contract code at
`ff07e2d`. Later commits change tests, docs and code comments only, so the results still apply.

| Crate | Critical | Medium | Enhancement |
|---|---|---|---|
| protection-pool | 192 | 23 | 49 |
| covered-registry | 0 | 7 | 3 |
| safu-account | 4 | 4 | 2 |
| cctp-adapter | 1 | 3 | 2 |
| cctp-common | 5 | 0 | 1 |

## Critical: integer overflow (202 results)

Every critical result is the `integer-overflow-or-underflow` lint on plain `+`, `-` and `*`. The
release profile sets `overflow-checks = true` (`contracts/Cargo.toml`), so an overflow aborts the call
and no state changes. Amounts are bounded well below `i128` limits by the pool cap and the stake
bounds. No change.

## Medium findings

| Lint | Where | Count | Verdict |
|---|---|---|---|
| Unsafe unwrap | pool `storage.rs` 205, 213, 235, 248, 256, 352 | 6 | Not reachable. Each reads an instance key (admin, oracle, co-signer, asset token, pool cap, guardian) that `__constructor` sets before any other call can run. Keys that can legitimately be missing use `Option` instead, for example `get_oracle_pubkey`. |
| Unsafe unwrap | registry `lib.rs` 125, 158, 160, 196, 202, 206 | 6 | Not reachable. Writer and pool are set in `__constructor`. |
| Unsafe unwrap | account `lib.rs` 186, 190, 194, 292 | 4 | Not reachable. Owner, home domain, pool and mode are set in `__constructor`. |
| Unsafe unwrap | adapter `lib.rs` 112, 234, 251 | 3 | Not reachable. Mode, pool and account WASM hash are set in `__constructor`. |
| Init instead of constructor | pool `admin.rs` 96 | 1 | False positive. `admin::initialize` is an internal function called only from the pool's `__constructor` (`lib.rs`), so setup runs in the deploy transaction itself and cannot be front-run. It also checks `AlreadyInitialized` and requires the admin's signature. |
| Ineffective extend TTL | pool `storage.rs` 640 | 1 | Intended. A revoked oracle approval is a temporary entry that must live exactly `REVOCATION_TTL_LEDGERS`, set once when it is written. Equal arguments are the point. |
| Front running | pool `stake.rs` 404, 444; `backer.rs` 441 | 3 | Not applicable. Each is a transfer of the caller's own money (emergency exit, staker yield, backer yield) at an amount the contract computed. No price, no swap, nothing a front-runner can move. |
| Missing new admin auth | pool `storage.rs` 209 (`set_admin`) | 1 | Already handled. `set_admin` is a storage helper. Its two callers are `__constructor` (the admin signs) and the governance `apply` step, which calls `require_auth` on the new admin first (`governance.rs`). The same holds for co-signer, guardian and oracle. |
| DoS: storage without access control | pool `backer.rs` 136, 167 | 2 | Bounded. The pending-deposit list belongs to one backer, needs that backer's signature, and is capped at `MAX_PENDING_DEPOSITS` (8). |
| DoS: storage without access control | pool `vault.rs` 349, 398, 565, 614, 771, 834, 1091 | 7 | False positive. These `push_back` calls build argument lists for a vault call in memory; nothing is written to storage. |
| DoS: unbounded operation | pool `governance.rs` 249-255 | 2 | Bounded. The loop compares 4 role addresses with each other: at most 6 comparisons. |
| Dynamic storage | registry `lib.rs` 146 | 1 | Bounded. A staker's wallet list is capped at `MAX_WALLETS_PER_STAKER` (3). |

## Enhancements (57)

- **Soroban version (5):** the lint asks for the newest SDK. The crates pin `soroban-sdk` 27.0.0, the
  version the contracts were built and tested on.
- **Assert violation (1):** `types.rs` 99 is a `const` assertion, checked when the code compiles.
  It never runs on-chain.
- **Storage change events (51):** flagged on the thin entry points in `lib.rs`. Those forward to a
  module function (`stake.rs`, `claim.rs`, `backer.rs`, `settings.rs`, `governance.rs` and so on), and
  the module function emits the event. To be checked one by one in the pre-audit review.

## Found while reviewing (not a Scout finding)

`GovChange::RegistryWriter` installs a new registry writer key without that key signing, unlike the
four role keys. A mistyped writer address would stop new covered-wallet registrations until another
governance change (7 days) replaces it. No money is at risk. Fixed on 2026-10-20: the new writer
now signs its own installation (`governance.rs`, test
`a_new_registry_writer_is_installed_only_with_its_own_signature`).
