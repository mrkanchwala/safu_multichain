# Mutation testing: the mutants no test kills, and why

`cargo-mutants` reports a mutant as missed when no test fails on it. Every missed mutant in the
protection pool has either been closed with a test or is listed here with the reason it cannot change
behaviour. This is our reasoning, written down so the auditor can check it. Runs and counts:
`docs/TESTING.md` section 3.

## Unreachable: an earlier check in the same call already covers it

| Mutant | Reason |
|---|---|
| `claim.rs` 1103 `!=` → `==` in `try_release_queued_claim` | Defence in depth. A wallet with a queued claim cannot get a second claim: `submit_claim` refuses a stake that already has one queued (`ClaimAlreadyQueued`), and an override that creates one forfeits the stake, which `queued_claim_dead` catches at the top of the same call. Kept so the invariant holds if a later path changes. |
| `claim.rs` 1111, 1112 `\|\|` → `&&` in `try_release_queued_claim` | The same three conditions (stake still points at this claim, amount above 0, not withdrawn) are checked by `queued_claim_dead` earlier in the same call, against the same storage. |
| `backer.rs` 409 guard → `true`, and `>` → `>=` in `collectable_view` | With no open request (`withdraw_amount == 0`), `limits` returns 0 due, so the view returns 0 either way. |
| `backer.rs` 276:41 `>` → `>=` in `limits` | `owed_all` is at least the request's own unpaid remainder, which is above 0 whenever `limits` runs (a fully paid request is closed first). |
| `settings.rs` 233, 234: delete the `PauseMaxSecs` / `PauseGapSecs` arms of `consistent_with_live` | The bounds already enforce the rule: a pause is at most 30 days and the gap at least 30 days, so "pause ≤ gap" can never fail. The arms stay in case the bounds change. |
| `backer.rs` 233 `>` → `>=` in `snap_values` | Capacity 0 means nothing is staked or backed, so no backer request exists to read it. |

## Same result either way

| Mutant | Reason |
|---|---|
| `stake.rs` 326 `>` → `>=` in `withdraw` | Banking 0 points adds 0. |
| `backer.rs` 139 `>` → `>=` in `settle_maturity` | With nothing matured, the update writes back the same values. |
| `vault.rs` 1022, 1027 `>` → `>=` in `credit_yield` | A zero bump adds 0 to the index and 0 to the reserve. |
| `points.rs` 50 `\|\|` → `&&` in `earned` | An early return. With either condition true, the formula below also gives 0 (zero amount, or no index growth). |
| `vault.rs` 993 `\|\|` → `&&` in `index_bumps` | An early return for a zero amount or zero capacity; the split below gives 0 bumps in both cases. |
| `vault.rs` 1118 `\|\|` → `&&` ×2 in `harvestable` | An early return. Nothing deployed, or no harvest clock yet, gives no growth to book below. |
| `vault.rs` 1161 `\|\|` → `&&` in `staker_yield_owed` | An early return. A zero index delta, a zero amount or a withdrawn stake each give 0 from the formula below. |
| `claim.rs` 458 `<` → `<=` in `watch_step` | When the cursor equals the head, resetting it to the head changes nothing. |
| `admin.rs` 160 `>` → `>=` in `pause` | With no earlier pause, the gap check compares against a start of 0, which any real ledger time passes. |
| `admin.rs` 223:31 `>` → `>=` in `suspend_stake` | For a stake never suspended, `0 + SUSPEND_MAX_SECONDS` is far below any real ledger time, so the check passes either way. |
| `storage.rs` 795 `>` → `>=` in `get_blocked_secs` | With no pause recorded, both branches give 0 blocked seconds. |
| `lib.rs` 236 `<` → `<=` in `migrate` | At the current version there are no migration steps; running the empty step set again writes the same version. |
| `lib.rs` 517 `>` → `>=` in `is_eligible` | A stake with amount 0 is always also withdrawn or forfeited, and both already make it ineligible. |

## Real, but hidden by another rule

| Mutant | Reason |
|---|---|
| `backer.rs` 377 `+` → `-` in `complete_withdrawal` | This asks the vault for principal minus yield instead of principal plus yield. When cash is short, `pull_for_payment` also refills the 20% cash buffer, and that refill covers the difference, so every test still pays in full. The mutant would only show when the buffer refill is smaller than the yield owed. |
