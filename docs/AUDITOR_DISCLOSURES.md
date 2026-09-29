# Auditor disclosures: known-by-design behaviour (v1)

For the security auditor, not end users. Items here are deliberate design choices
with measured consequences, disclosed up front so they are not reported as
undiscovered findings. Each one points to the test or script that measures it.

## 1. Pool liquidity model: stake inflow > payouts (founder decision, 2026-09-22)

**Behaviour.** Stakers are always repaid their full principal (plus their yield
share); no staker's balance is ever reduced after a claim. A claim larger than the
claimant's own forfeited stake is paid from pooled liquidity. The contract does not
assign that shortfall to any staker. It is covered over time by new stake, the
protocol's yield share, and backer capital.

**Consequence.** If the pool winds down with no new money, the last withdrawer can
hit `InsufficientLiquidity` and wait until new stake or backer capital arrives.
Solvency is enforced on admission (`total_allocated + entitlement <= total_staked`,
daily admission/payout caps), not by per-staker loss accounting.

**Why accepted.** SAFU is designed around liquidity over time: the 90-day gate,
7-day cooldown, 45-day vesting and the daily payout cap spread every claim out;
claims queue rather than fail; new stake is always accepted; backers are the last
line of defence.

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

## 2. Adjustable settings instead of an upgrade path

No WASM upgrade exists. A fixed set of numbers is adjustable within hard bounds
(`src/settings.rs`) via admin proposal → co-signer approval of the same value →
7-day public timelock → execution, with events at every step. Tier ceilings, the
90-day gate, the 30-day claim window and every funds-movement path are constants.
Settings a live claim depends on (cooldown, vesting) are fixed on the claim at
activation. Tests: `src/test/settings_tests.rs`.

## 3. Key-loss-safe governance: 2 of 3 roles, 7-day delay, 90-day recovery (founder decision, 2026-09-23)

**Behaviour.** Three roles: admin, co-signer, guardian (`src/governance.rs`). Every
change that grants power or moves where money goes (the three roles, the oracle
address + attestation key together, vault, treasury, vault deploy ceiling, the
registry's writer key) needs one role to propose, a second role to approve the same
change, then 7 days in public (`GOV_DELAY_SECONDS`) before anyone can execute it.
Cancelling also takes two roles. The role a change replaces never counts: it cannot
propose, approve or cancel its own replacement.

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

## 4. Admin alone can pause the pool and suspend a stake (/cso M-1, 2026-09-24: disclose, no change)

**Behaviour.** `pause()` and `suspend_stake()` need the admin only (`src/admin.rs`).
A pause lasts at most 30 days (`PAUSE_MAX_SECONDS`) and ends on its own, but the admin
can call `pause()` again with no cooldown. So one admin key, if stolen, can keep the
pool paused or suspend stakes until the other two roles replace the admin, which
takes 7 days through governance (section 3).

**Consequence.** A stolen admin key can delay stakes, claims and payouts by about 7
days. It cannot move money, change who gets paid, or raise a payout: while paused,
`emergency_exit` stays open, and a suspended staker can still withdraw principal.

**Why accepted.** Pausing fast, with one key, is the point of a circuit breaker. The
delay is short next to a claim's own timeline (7-day cooldown, then 45-day vesting),
and replacing the admin needs no contract change.

**Tests.** `src/test/admin_tests.rs`, `src/test/governance_tests.rs`.

## 5. The pool rebalances with its yield vault inside user calls (founder decision, 2026-09-24)

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
  full (founder decision 2026-09-24: backers exist to build confidence, they never get burnt).
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
