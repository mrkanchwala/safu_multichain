# Auditor disclosures: known-by-design behaviour (v1)

For the security auditor, not end users. Items here are deliberate design choices
with measured consequences, disclosed up front so they are not reported as
undiscovered findings. Each one points to the test or script that measures it.

## 1. Pool liquidity model: stake inflow > payouts (design decision, 2026-09-22)

**Behaviour.** Stakers are always repaid their full principal (plus their yield
share); no staker's balance is ever reduced after a claim. A claim larger than the
claimant's own forfeited stake is paid from pooled liquidity. The contract does not
assign that shortfall to any staker. It is covered over time by new stake, the
protocol's yield share, and backer capital.

**Consequence.** If the pool winds down with no new money, the last withdrawer can
hit `InsufficientLiquidity` and wait until new stake or backer capital arrives.
Solvency is enforced on admission (`total_allocated + entitlement <= capacity`, where
capacity is `total_staked + total_backed`, plus daily admission/payout caps), not by
per-staker loss accounting.

**Why accepted.** SAFU is designed around liquidity over time: the 90-day gate,
7-day cooldown, 45-day vesting and the daily payout cap spread every claim out;
claims queue rather than fail; new stake is accepted up to the pool cap; backers are
the last line of defence.

**Measured.**
- Contract tests `src/test/pool_solvency_tests.rs` ($100K cap, 1,000 stakers, 5% APY,
  50/50 split): no claims → everyone repaid with yield; 5 × $500 Tier C claims
  absorbed in full; one more → the last withdrawers wait, and one new stake repays them.
- Simulation `scripts/sim/solvency_sim.py` (120 liquidity scenarios incl. a panic
  stress: withdrawals ×3 and zero new stake for 60 days): zero withdrawal waits,
  zero late claim payments, 0.25–3% hacks/yr, all tier mixes, with or without seed.
- Break point `scripts/sim/steady_state.py` (pool size held constant, only the hack
  rate raised; 10 years):

  | Hacks / yr (% of stakers) | First cash shortfall |
  |---|---|
  | 1% | none within 10 years |
  | 3% | ~6–9 years |
  | 5% | ~4–5 years |
  | 10% | ~2 years |
  | 20%+ | ≤ ~1 year |

  Real-world rate for comparison: ~0.15–0.35%/yr (Chainalysis / Scam Sniffer 2025).
  Operational rule: backer capital is lined up once the realised staker hack rate
  exceeds ~3%/yr.

## 2. Adjustable settings, and an upgrade path behind the same governance (updated 2026-10-20)

**The pool is upgradable (since 2026-10-16).** A code upgrade is a governance change
(`GovChange::Upgrade(wasm_hash)`, `src/governance.rs`): one role proposes, a second role approves,
it waits `GOV_DELAY_SECONDS` (7 days) in public, then anyone executes. An upgrade can never use the
90-day single-role recovery path. Storage is versioned (`get_storage_version`, `migrate`), so a new
WASM reads the old entries. Trust limit, stated plainly: **2 of the 3 roles = full control after 7
days.** Stakers can `emergency_exit` during those 7 days, also while the pool is paused.

**Settings** (`src/settings.rs`): a fixed list of numbers, each inside hard bounds, changes only by
admin proposal → co-signer approval of the same value → 7-day public timelock → execution, with an
event at every step. This now includes the tier ceilings (A ≥ B ≥ C, 1x–20x; a cut applies to new
stakes only, a raise to everyone), the claim time gate (floor 90 days, can only go up) and the claim
window. Values a stake or claim depends on are snapshotted onto it, so a change never reaches someone
already in. Tests: `src/test/settings_tests.rs`, `src/test/governance_tests.rs`.

## 3. Key-loss-safe governance: 2 of 3 roles, 7-day delay, 90-day recovery (design decision, 2026-09-23)

**Behaviour.** Three roles: admin, co-signer, guardian (`src/governance.rs`). Every
change that grants power or moves where money goes (the three roles, the oracle
address + attestation key together, vault, treasury, vault deploy ceiling, the
registry's writer key, a code upgrade) needs one role to propose, a second role to approve the same
change, then 7 days in public (`GOV_DELAY_SECONDS`) before anyone can execute it.
Cancelling also takes two roles. The role a change replaces never counts: it cannot
propose, approve or cancel its own replacement. A new admin, co-signer, guardian,
oracle or registry writer key must sign its own installation, so a mistyped address cannot be installed.

**Last-resort recovery.** If two roles are lost, the remaining one can propose a role
replacement alone. It waits 90 days (`RECOVERY_DELAY_SECONDS`), and any other role
still working, including the one being replaced, can cancel it. Only role
replacements can use this path.

**Accepted trade-off.** One role acting alone can take over after 90 days if nobody
else objects. One role alone (instead of two) was chosen deliberately: with 2 of 3
here, a single honest role could not stop a thief's recovery once the third role is
lost; the scenario sweep put that at ~20x the takeover risk. Not recoverable: all
three roles lost, or one honest role against one thief (each cancels the other). In
both cases stakers can still withdraw and collect claims, and a pause expires on its
own (section 4), so no money is frozen forever.

**Tests.** `src/test/governance_tests.rs`.

## 4. Admin alone can pause the pool and suspend a stake, within limits (security review M-1, 2026-09-24; limits added 2026-10-16)

**Behaviour.** `pause()` and `suspend_stake()` need the admin only (`src/admin.rs`).
A pause lasts at most `PauseMaxSecs` (default 30 days) and ends on its own, and the next
pause can start only `PauseGapSecs` (at least 30 days) after the last one ended. A
suspension by the admin alone lapses after 30 days (`SUSPEND_MAX_SECONDS`); suspending
again within that length needs a second role (`renew_suspension`). So one stolen admin
key can delay things once, and the other two roles replace the admin in 7 days
through governance (section 3).

**Consequence.** A stolen admin key can delay stakes, claims and payouts by about 7
days. It cannot move money, change who gets paid, or raise a payout: while paused,
`emergency_exit` stays open, and a suspended staker can still withdraw principal.

**Why accepted.** Pausing fast, with one key, is the point of a circuit breaker. The
delay is short next to a claim's own timeline (7-day cooldown, then 45-day vesting),
and replacing the admin needs no contract change.

**Tests.** `src/test/admin_tests.rs`, `src/test/governance_tests.rs`.

## 5. The pool rebalances with its yield vault inside user calls (design decision, 2026-09-24)

**Behaviour.** Up to 80% of capacity (`deploy_bps`) can sit in the DeFindex vault. The pool
keeps that line itself, with no keeper (`src/vault.rs`, module doc rule 3):
- **Money out** (`withdraw`, `claim_stream`, `emergency_exit`, `complete_backer_withdrawal`):
  if cash is short, `pull_for_payment` redeems the shortfall plus a refill to the 20% buffer,
  then pays. If the vault can't give that much, it tries the shortfall alone.
- **Money in** (`stake`, `back`, `mature_backing`): `push_idle` deposits idle cash above the
  buffer, once it is at least 1% of capacity (`AUTO_PUSH_MIN_BPS`).
- Both use `try_` vault calls, so a vault failure leaves no state behind. A failed pull ends
  as `InsufficientLiquidity`; a failed push is skipped and the stake still succeeds (a CCTP
  mint must never revert). No new function, role or authority; the permissionless
  `ensure_liquidity` / `auto_deploy_liquidity` stay.

**Consequences.**
- A pull accepts up to 5% below book value (`MAX_REBALANCE_SLIPPAGE_BPS`, enforced by the
  vault's own `min_amounts_out`). A realised loss is marked down in `total_staked` pool-wide,
  never on the exiting staker's own payout, and never on backers: backers are always repaid in
  full (design decision 2026-09-24: backers exist to build confidence, they never get burnt).
  So during a vault loss, early exits are paid in
  full and the loss stays with those who remain (a bank-run shape). This exists without the
  in-path pull too (anyone can call `ensure_liquidity`); the pull only makes exiting faster.
- A vault loss beyond 5%: the vault refuses, every short payment returns
  `InsufficientLiquidity`, and only the admin can redeem at a lower floor
  (`provide_liquidity`). That is deliberate: the pool never dumps its position at a large loss
  automatically.
- A push that gets fewer shares than the reference rate still keeps the stake and records the
  real shares received (`PushBelowFloor` event), so accounting stays true.
- Cash-paid exits can leave the vault above 80% until the next pull (which refills) or an
  `ensure_liquidity` call. Harmless: solvency is unaffected.

**Why accepted.** Stakers and victims should never wait on SAFU to act before they can be paid.

**Tests.** `src/test/inpath_liquidity_tests.rs` (incl. adversarial: churn, donations, paused
bank run, loss during a run, daily-cap bypass, theft of incoming money), `src/test/d2_vault_tests.rs`.

## 6. The claim line: order, never speed (design decision, 2026-10-16)

**Behaviour.** A claim the pool cannot admit today (daily admission cap, capacity, cash, or the
oracle's daily throttle) joins a line (`src/claim.rs`, `queue_front`, `try_release_queued_claim`).
Queued claims are released strictly in the order they joined, and a new claim joins the back while
anyone waits, even if there would be room for it alone. Each streaming claim gets its own pro-rata
share of the day's payout cap by what it is still owed; an unused share stays vested and carries
forward. No expiry clock runs on a day the pool could not pay; a queued claim is cleared only once
its stake is gone.

**Consequence.** A small claim can wait behind a large one. That is the price of a line nobody can
jump by calling first or by watching the mempool.

**Tests.** `src/test/b1_tests.rs`, `src/test/line_and_watch_tests.rs`.

## 7. Upkeep runs inside user calls, capped (design decision, 2026-10-19)

**Behaviour.** There is no keeper. At the end of every user transaction `upkeep` releases the head
of the line if there is room, moves due claims forward and clears finished entries from an on-chain
watch list: at most `UPKEEP_WATCH_PER_CALL` (2) entries per call, skipping at most a fixed number of
empty slots. Each step checks before it writes and is skipped if it cannot run, so upkeep never fails
the caller. It is skipped while paused.

**Consequence.** On a quiet day, bookkeeping waits for the next transaction. Nothing a user is owed
depends on it: the claim functions apply the same due checks themselves.

**Tests.** `src/test/b1_tests.rs`, `src/test/line_and_watch_tests.rs`.

## 8. Rounding reserve (design decision, 2026-10-19)

**Behaviour.** DeFindex locks a small number of shares at a vault's first deposit, and Blend rounds
by a unit. Either can leave the last staker a few units short. The vault setup script makes the
first deposit from a separate seeder wallet, so the locked shares are never the pool's. On top of
that the pool holds a rounding reserve that anyone can fund and nothing can take out: a payment
still short after the vault pays may draw up to 10,000 units (0.001 USDC) from it. The reserve is not
pool cash, capacity or treasury surplus.

**Tests.** `src/test/b1_tests.rs`, `src/test/inpath_liquidity_tests.rs`.

## 9. After a paid claim, that address never stakes again (design decision, 2026-10-19)

**Behaviour.** A paid claim forfeits the whole stake, and the address is blocked from staking
again (`AddressHasApprovedClaim`). To stake again the owner uses a fresh address with new covered
wallets; points start at zero. Without an approved claim, the same address can withdraw and stake
again as usual.

**Why.** The covered-wallet registry binds a wallet to its staker for good, and coverage only checks
for a live stake. A same-address re-stake would cover the drained wallet again.
