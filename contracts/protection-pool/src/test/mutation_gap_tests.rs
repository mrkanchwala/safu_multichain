//! Tests added 2026-07-15 to close mutation-testing gaps (cargo-mutants,
//! first full run: 464 mutants, 60 missed). Each test names the mutant(s)
//! it kills by file:line. The 9 mutants NOT covered here are provably
//! equivalent (no observable behavior change), documented with
//! justification in `.cargo/mutants.toml` at the workspace root, not
//! silently skipped.
//!
//! The common thread in what the original suite missed: line coverage was
//! 96%+ but boundary values (`>` vs `>=` at exact thresholds) and exact
//! arithmetic results (points formula, allocation release math) were
//! executed without being pinned by assertions. These tests assert exact
//! numbers at exact boundaries.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::error::PoolError;
use crate::types::{
    ClaimStatus, BPS_DENOMINATOR, COOLDOWN_LEDGERS, LEDGERS_PER_DAY, MIN_STAKE_BPS,
    PENALTY_LOCK_LEDGERS, TIER_C_RATIO, TIME_GATE_LEDGERS, VESTING_LEDGERS,
};

/// A small entitlement that fits every pool these tests build.
const ENTITLEMENT: i128 = STROOPS_PER_UNIT;
const TIER_B: u32 = 2;
const TIER_C: u32 = 3;

/// Test-only re-derivation of the on-chain claim id, same as the helper
/// in override_tests.rs (kept local: test modules don't export helpers).
fn claim_id_for(
    env: &soroban_sdk::Env,
    wallet: &Address,
    hash: &soroban_sdk::BytesN<32>,
) -> soroban_sdk::BytesN<32> {
    use soroban_sdk::xdr::ToXdr;
    let mut buf = wallet.to_xdr(env);
    buf.append(&soroban_sdk::Bytes::from_array(env, &hash.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}

/// Full 2-of-2 override (admin then coSigner, identical params).
fn do_override(
    env: &soroban_sdk::Env,
    s: &Setup<'_>,
    wallet: &Address,
    hash: &soroban_sdk::BytesN<32>,
    entitlement: i128,
    tier: u32,
) {
    let _ = env;
    s.client
        .approve_override(&s.admin, wallet, hash, &entitlement, &tier);
    s.client
        .approve_override(&s.co_signer, wallet, hash, &entitlement, &tier);
}

// -----------------------------------------------------------------------
// Points formula: exact values at every day-tier boundary.
// Kills stake.rs:82:19 (>→==), 82:38 (−→+, −→/), 83:19 (>→==),
// 83:29 (−→+, −→/), 84:36 (+→−), 84:41 (*→/), 84:47 (+→−, +→*),
// 84:52 (*→/), and lib.rs:151 (get_stake→None, via the live-computed
// points path needing a real record).
// points = (d1*100 + d2*120 + d3*150 + d4*200) * amount / max_stake,
// The raw day-tier sums below are pure calendar math, worked out by hand and
// independent of the contract. The stake factor is amount / max_stake, taken
// from the real stake bounds.
// -----------------------------------------------------------------------

#[test]
fn points_formula_exact_values_at_all_day_tier_boundaries() {
    let cases: [(u32, i128); 10] = [
        (45, 4_500),   // inside tier 1
        (90, 9_000),   // tier-1/2 boundary: d2 still 0
        (91, 9_120),   // first tier-2 day
        (135, 14_400), // mid tier 2
        (180, 19_800), // tier-2/3 boundary
        (181, 19_950), // first tier-3 day
        (250, 30_300), // mid tier 3
        (365, 47_550), // tier-3/4 boundary
        (366, 47_750), // first tier-4 day
        (400, 54_550), // deep tier 4
    ];
    for (days, raw_points) in cases {
        let expected = raw_points * MID_STAKE / MAX_STAKE;
        let env = new_env();
        let s = setup(&env);
        let (staker, _ben) = staked_wallet(&env, &s);
        advance_days(&env, days);
        assert_eq!(
            s.client.get_points_balance(&staker),
            expected,
            "points mismatch at day {}",
            days
        );
    }
}

// -----------------------------------------------------------------------
// lib.rs views.
// -----------------------------------------------------------------------

/// Kills lib.rs:151 (get_stake → None).
#[test]
fn get_stake_returns_real_record() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let record = s.client.get_stake(&staker).expect("record must exist");
    assert_eq!(record.amount, MID_STAKE);
    assert!(!record.withdrawn);
}

/// Kills lib.rs:206 (is_paused → true AND is_paused → false).
#[test]
fn is_paused_tracks_pause_state_both_ways() {
    let env = new_env();
    let s = setup(&env);
    assert!(!s.client.is_paused());
    s.client.pause();
    assert!(s.client.is_paused());
    s.client.unpause();
    assert!(!s.client.is_paused());
}

// -----------------------------------------------------------------------
// stress_cap / daily-entitlement accumulation (submit_claim admission).
// -----------------------------------------------------------------------

/// Five stakers. First claim brings utilization to EXACTLY the first band
/// edge; the next claim the same real day must hit the tightened stress cap.
/// Kills claim.rs:119:43 (*→/ in stress_cap's utilization),
/// 120:39 (<→<= at the 2000 boundary), 293:25 (+→−, +→* in the
/// daily-entitlement accumulation), and 150:31 (/→* in current_day,
/// the ledger advance between the claims shifts the timestamp within
/// the same real day; a mutated day value would wrongly reset the
/// daily counter).
#[test]
fn stress_cap_tightens_at_exactly_20_percent_utilization() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE);
    for _ in 0..3 {
        staked_wallet_amount(&env, &s, MAX_STAKE);
    }
    let total = 5 * MAX_STAKE;
    // Utilisation of exactly the first band edge, which fits under the
    // zero-utilisation cap.
    let first = bps_of(total, BAND_1_UTILISATION_BPS);
    assert!(first <= expected_stress_cap(&s));
    submit_claim_signed(&env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &first, &TIER_C, &now_ts(&env));
    // Utilisation is now exactly on the edge, so the rate must already have
    // dropped. Same real day (a few seconds later), different ledger.
    assert_eq!(s.client.get_total_allocated() * BPS_DENOMINATOR / total, BAND_1_UTILISATION_BPS);
    advance_ledgers(&env, 100);
    // Today's total plus this entitlement is over the tightened cap → must be
    // blocked. T3 (2026-08-24): this queues (ClaimStatus::Reserved) instead of
    // hard-erroring, the boundary being tested is unchanged.
    let entitlement = MIN_STAKE;
    assert!(first + entitlement > expected_stress_cap(&s));
    let result = try_submit_claim_signed(&env, &s, &s.oracle, &w2, &tx_hash(&env, 2), &entitlement, &TIER_C, &now_ts(&env));
    let claim_id = result.unwrap().unwrap();
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.entitlement, entitlement);
}

/// Day 1 takes utilisation to the first band edge; days 2 to 4 each fill the
/// day's stress cap exactly, walking utilisation up to the second band edge.
/// Returns the last day's fill. The fill comes from the independent oracle,
/// never from the contract, so a mutated cap cannot make itself pass.
fn walk_to_second_band_edge(
    env: &soroban_sdk::Env,
    s: &Setup<'_>,
    wallets: &[Address; 4],
) -> i128 {
    let total = s.client.get_total_staked();
    submit_claim_signed(
        env, s, &s.oracle, &wallets[0], &tx_hash(env, 1),
        &bps_of(total, BAND_1_UTILISATION_BPS), &TIER_C, &now_ts(env),
    );
    let mut fill = 0;
    for (i, wallet) in wallets.iter().enumerate().skip(1) {
        advance_days(env, 1);
        fill = expected_stress_cap(s);
        submit_claim_signed(
            env, s, &s.oracle, wallet, &tx_hash(env, (i + 1) as u8), &fill, &TIER_C, &now_ts(env),
        );
    }
    fill
}

/// Days 1–4: fill the daily stress cap EXACTLY each day, walking
/// utilization through 2000→3000→4000→5000 bps. All four must succeed,
/// any mutant that tightens the rate early (e.g. <→== / <→> at the
/// 5000-bps branch, which would misprice utilization 3000/4000 at
/// 300 bps) panics mid-walk and is caught.
/// Kills claim.rs:122:31 (<→==, <→>) and 270:38 (>→>= via the exact
/// daily fills).
#[test]
fn stress_cap_exact_daily_fills_walk_utilization_to_50_percent() {
    let env = new_env();
    let s = setup(&env);
    let wallets = [
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
    ];
    staked_wallet_amount(&env, &s, MAX_STAKE);
    // Every fill is admitted (the walk panics on a refusal), landing exactly on
    // the second band edge.
    walk_to_second_band_edge(&env, &s, &wallets);
    assert_eq!(
        s.client.get_total_allocated(),
        bps_of(s.client.get_total_staked(), BAND_2_UTILISATION_BPS)
    );
}

/// Day 5 of the walk above: utilization is EXACTLY 5000 bps → rate must
/// be the lowest band's (the `< 5_000` branch must NOT admit 5000 itself).
/// Kills claim.rs:122:31 (<→<= at the 5000 boundary).
#[test]
fn stress_cap_rate_drops_at_exactly_50_percent_utilization() {
    let env = new_env();
    let s = setup(&env);
    let wallets = [
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
        staked_wallet_amount(&env, &s, MAX_STAKE).0,
    ];
    let (w5, _b5) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let last_fill = walk_to_second_band_edge(&env, &s, &wallets);
    // Utilization exactly on the second band edge → the cap must already have
    // dropped below the previous band's. An entitlement one unit over it must
    // be blocked. T3 (2026-08-24): queues instead of hard-erroring, same
    // boundary logic, different observable outcome (see comment above).
    advance_days(&env, 1);
    let cap = expected_stress_cap(&s);
    assert!(cap < last_fill, "the cap must have dropped at the edge");
    let entitlement = cap + 1;
    let result = try_submit_claim_signed(&env, &s, &s.oracle, &w5, &tx_hash(&env, 5), &entitlement, &TIER_C, &now_ts(&env));
    let claim_id = result.unwrap().unwrap();
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.entitlement, entitlement);
}

/// Entitlement that EXACTLY fills both the solvency gap and the daily
/// stress cap must be admitted (strict `>` on both checks).
/// Kills claim.rs:264:38 (>→>= solvency) and 270:38 (>→>= stress cap).
#[test]
fn submit_claim_exact_solvency_and_stress_fill_succeeds() {
    let env = new_env();
    let s = setup(&env);
    let (wa, _ba) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (wb, _bb) = staked_wallet_amount(&env, &s, MAX_STAKE);
    // Override forfeits wa's stake: staked halves, most of it now allocated.
    let allocated = bps_of(MAX_STAKE, EXACT_FILL_UTILISATION_BPS);
    do_override(&env, &s, &wa, &tx_hash(&env, 1), allocated, TIER_C);
    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    // Next day: at this utilisation the lowest band's cap equals the solvency
    // gap, so an entitlement of that size fills BOTH exactly, must pass.
    advance_days(&env, 1);
    let gap = MAX_STAKE - allocated;
    assert_eq!(expected_stress_cap(&s), gap);
    submit_claim_signed(&env, &s, &s.oracle, &wb, &tx_hash(&env, 2), &gap, &TIER_C, &now_ts(&env));
    assert_eq!(s.client.get_total_allocated(), MAX_STAKE);
}

// -----------------------------------------------------------------------
// claim_stream boundaries and math.
// -----------------------------------------------------------------------

/// At EXACTLY cooldown_ends the cooldown check must pass (strict `<`),
/// the call then fails on "nothing vested yet" (elapsed = 0), which is a
/// DIFFERENT panic than "cooldown not passed".
/// Kills claim.rs:384:19 (<→<=).
#[test]
fn claim_stream_at_exact_cooldown_end_passes_cooldown_check() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    do_override(&env, &s, &staker, &hash, ENTITLEMENT, TIER_C);
    advance_ledgers(&env, COOLDOWN_LEDGERS); // now == cooldown_ends_ledger exactly
    let result = s
        .client
        .try_claim_stream(&claim_id_for(&env, &staker, &hash), &ben);
    assert_eq!(result, Err(Ok(PoolError::NothingVested)));
}

/// Second stream pays EXACTLY the newly-vested delta, not vested+streamed.
/// Kills claim.rs:401:34 (−→+ in claimable).
#[test]
fn claim_stream_second_call_pays_exactly_the_delta() {
    // Derives the vested fraction from VESTING_LEDGERS
    // itself (a quarter, then another quarter) instead of a hand-picked day
    // count, so this stays correct at whatever the constant is set to.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    // Both streams land on the same real day, so half the entitlement must fit
    // under that day's payout cap or the cap, not the delta, would be measured.
    let entitlement = MID_STAKE / 20;
    assert!(entitlement / 2 <= outflow_cap_oracle(MID_STAKE, entitlement));
    do_override(&env, &s, &staker, &hash, entitlement, TIER_C);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS);
    let quarter = (VESTING_LEDGERS / 4) as i128;
    let total = VESTING_LEDGERS as i128;
    advance_ledgers(&env, VESTING_LEDGERS / 4); // 1/4 of vesting elapsed
    let first_expected = entitlement * quarter / total;
    assert_eq!(s.client.claim_stream(&claim_id, &ben), first_expected);
    advance_ledgers(&env, VESTING_LEDGERS / 4); // 1/2 of vesting elapsed
    let second_expected = entitlement * (2 * quarter) / total - first_expected;
    assert_eq!(s.client.claim_stream(&claim_id, &ben), second_expected);
}

/// Cancelling a partially-streamed claim releases EXACTLY the unstreamed
/// remainder from total_allocated.
/// Kills claim.rs:450:40 (−→+ in unstreamed).
///
/// CHANGED 2026-07-22 (bug 1 fix): the old expected value here (the streamed
/// amount) was pinning the BUG, claim_stream never used to release its own
/// transferred amount from total_allocated, so the streamed part sat there
/// forever as phantom allocation even after cancel released the remainder.
/// Now claim_stream releases its own transfer as it happens, so
/// total_allocated is already down to `entitlement - streamed` by the time
/// cancel_claim runs; cancel then correctly releases that same remainder
/// (unchanged formula, now operating on an already-accurate total_allocated
/// instead of an inflated one), ending at exactly 0. Kill-coverage for the
/// `−→+` mutant is unaffected: the formula itself didn't change, only what it
/// was released FROM did.
#[test]
fn cancel_partially_streamed_claim_releases_exact_remainder() {
    // Vested fraction derived from VESTING_LEDGERS.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    let entitlement = ENTITLEMENT;
    do_override(&env, &s, &staker, &hash, entitlement, TIER_C);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS);
    advance_ledgers(&env, VESTING_LEDGERS / 4);
    let streamed = entitlement * (VESTING_LEDGERS / 4) as i128 / VESTING_LEDGERS as i128;
    assert_eq!(s.client.claim_stream(&claim_id, &ben), streamed);
    assert_eq!(s.client.get_total_allocated(), entitlement - streamed); // bug 1: released as-streamed
    s.client.cancel_claim(&claim_id);
    // Cancel releases the remaining reservation for this claim, nothing
    // left allocated for it at all.
    assert_eq!(s.client.get_total_allocated(), 0);
}

// -----------------------------------------------------------------------
// dynamic_outflow_bps: exact utilization boundaries (payout side).
// -----------------------------------------------------------------------

/// Utilization EXACTLY 2000 bps against the cap base → rate must already
/// be 300 (the `< 2_000` branch must not admit 2000 itself).
/// Kills claim.rs:140:24 (<→<=).
#[test]
fn outflow_rate_drops_at_exactly_20_percent_utilization() {
    // Two MAX_STAKE stakers give a cap base of 2 * MAX_STAKE. The entitlement
    // sits at exactly the first band edge of that base, so the payout cap must
    // already be the second band's.
    let env = new_env();
    let s = setup(&env);
    let anchor = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor, &MAX_STAKE, &anchor_ben);
    let (staker, ben) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let cap_base = 2 * MAX_STAKE;
    let entitlement = bps_of(cap_base, BAND_1_UTILISATION_BPS);
    advance_ledgers(&env, TIME_GATE_LEDGERS);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &entitlement,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // fully vested
    let expected = outflow_cap_oracle(cap_base, entitlement);
    assert!(expected < bps_of(cap_base, OUTFLOW_RATE_BAND_1_BPS), "the rate must have dropped");
    assert_eq!(s.client.claim_stream(&claim_id, &ben), expected);
}

/// Utilization EXACTLY 5000 bps → rate must already be 100.
/// Kills claim.rs:142:31 (<→<=).
#[test]
fn outflow_rate_floor_at_exactly_50_percent_utilization() {
    // Two MAX_STAKE stakers give a cap base of 2 * MAX_STAKE. The entitlement
    // sits at exactly the second band edge of that base. Half the vesting has
    // elapsed, which is far more than the payout cap, so the cap (not the
    // vesting schedule) is what limits this call.
    let env = new_env();
    let s = setup(&env);
    let anchor = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor, &MAX_STAKE, &anchor_ben);
    let (staker, ben) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let hash = tx_hash(&env, 1);
    let cap_base = 2 * MAX_STAKE;
    let entitlement = bps_of(cap_base, BAND_2_UTILISATION_BPS);
    do_override(&env, &s, &staker, &hash, entitlement, TIER_B);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 2);
    let expected = outflow_cap_oracle(cap_base, entitlement);
    assert!(expected < entitlement / 2, "the cap must bind, not the vesting");
    assert!(expected < bps_of(cap_base, OUTFLOW_RATE_BAND_2_BPS), "the rate must have dropped");
    assert_eq!(s.client.claim_stream(&claim_id, &ben), expected);
}

// -----------------------------------------------------------------------
// execute_override: status gate, release math, deadlines, boundaries.
// -----------------------------------------------------------------------

/// Re-executing an override on a still-Active claim must release the
/// prior reservation before re-adding: allocation ends at exactly
/// ENTITLEMENT, and the withdrawn-branch deadlines are exact.
/// Kills claim.rs:578:44 (||→&&), 578:21 (==→!= via the Active path),
/// 641:49 and 642:64 (+→− in the withdrawn-branch deadlines).
#[test]
fn override_reexecution_releases_prior_reservation_exactly() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    do_override(&env, &s, &staker, &hash, ENTITLEMENT, TIER_C);
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT);
    advance_ledgers(&env, COOLDOWN_LEDGERS);
    do_override(&env, &s, &staker, &hash, ENTITLEMENT, TIER_C);
    // Released then re-added: NOT doubled.
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT);
    // Withdrawn-branch deadlines: fresh from "now" (re-execution ledger).
    let now = env.ledger().sequence();
    let claim = s.client.get_claim(&claim_id_for(&env, &staker, &hash)).unwrap();
    assert_eq!(claim.cooldown_ends_ledger, now + crate::types::COOLDOWN_LEDGERS);
    assert_eq!(
        claim.vesting_ends_ledger,
        now + crate::types::COOLDOWN_LEDGERS + crate::types::VESTING_LEDGERS
    );
}

/// Overriding a wallet whose prior claim was CANCELLED must NOT release
/// anything (cancel already did), asserted with a second wallet's live
/// reservation present, so a wrong release visibly deducts from it
/// instead of vanishing into the .max(0) clamp.
/// Kills claim.rs:578:21 and 578:56 (==→!= via the Cancelled path).
#[test]
fn override_after_cancel_does_not_touch_other_reservations() {
    let env = new_env();
    let s = setup(&env);
    let (wx, _bx) = staked_wallet(&env, &s);
    let (wy, _by) = staked_wallet(&env, &s);
    // Live reservation on wx (PendingTime, no forfeiture).
    submit_claim_signed(&env, &s, &s.oracle, &wx, &tx_hash(&env, 1), &ENTITLEMENT, &TIER_C, &now_ts(&env));
    // wy: claim then cancel (its reservation already released by cancel).
    // Next day, so the oracle's per-day claim count does not queue it.
    advance_days(&env, 1);
    let hash_y = tx_hash(&env, 2);
    let claim_y = submit_claim_signed(&env, &s, &s.oracle, &wy, &hash_y, &ENTITLEMENT, &TIER_C, &now_ts(&env),
    );
    s.client.cancel_claim(&claim_y);
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT); // wx's only
    // Override re-targets wy (prior status: Cancelled). Must add wy's
    // new reservation WITHOUT releasing anything, wx's stays intact.
    advance_ledgers(&env, PENALTY_LOCK_LEDGERS + 1); // penalty lock from the cancel clears
    do_override(&env, &s, &wy, &hash_y, ENTITLEMENT, TIER_C);
    assert_eq!(s.client.get_total_allocated(), 2 * ENTITLEMENT);
}

/// Re-execution after a partial stream: release is exactly
/// entitlement − streamed, then the new entitlement re-adds in full.
/// Kills claim.rs:579:44 (−→+) and 581:64 (−→+, −→/).
///
/// CHANGED 2026-07-22 (bugs 1 + 3 fixes): the old expected value was pinning
/// TWO bugs at once: bug 1 (claim_stream never released its own transfer, so
/// total_allocated was still the full entitlement going into the re-execution
/// instead of `entitlement - streamed`) and bug 3 (the re-executed claim's
/// `streamed` was hard-reset to 0, letting the beneficiary re-collect the FULL
/// new entitlement on top of what was already paid, an overpayment). Both
/// fixed: total_allocated is `entitlement - streamed` before re-execution,
/// releases that (→0), re-adds the full entitlement, and the new claim record
/// carries `streamed` forward, so only the remainder is still collectible.
#[test]
fn override_reexecution_after_partial_stream_exact_release_math() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    let entitlement = ENTITLEMENT;
    do_override(&env, &s, &staker, &hash, entitlement, TIER_C);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS);
    advance_ledgers(&env, VESTING_LEDGERS / 4);
    let streamed = entitlement * (VESTING_LEDGERS / 4) as i128 / VESTING_LEDGERS as i128;
    assert_eq!(s.client.claim_stream(&claim_id, &ben), streamed);
    assert_eq!(s.client.get_total_allocated(), entitlement - streamed); // bug 1: already net of streamed
    // Re-execute same params: release the current remainder (→0), re-add
    // the full entitlement (→ entitlement), not entitlement + streamed.
    do_override(&env, &s, &staker, &hash, entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);
    // Bug 3 regression: streamed carried forward, not reset to 0, the
    // beneficiary can only collect the remainder, not the full entitlement
    // again.
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.streamed, streamed);
}

/// An override entitlement EXACTLY at the tier cap AND exactly at the
/// solvency limit must execute (strict `>` on both checks).
/// Kills claim.rs:598:24 (>→>=) and 618:42 (>→>=).
#[test]
fn override_exact_tier_cap_and_solvency_fill_succeeds() {
    let env = new_env();
    let s = setup(&env);
    let (wa, _ba) = staked_wallet_amount(&env, &s, MAX_STAKE);
    // As many stakers as the tier C ratio, so the pool total equals wa's cap.
    for _ in 1..TIER_C_RATIO {
        staked_wallet_amount(&env, &s, MAX_STAKE);
    }
    // wa's tier C cap is TIER_C_RATIO x its stake, which here is also the whole
    // pool: an entitlement of that size is exactly AT the tier cap and exactly
    // fills solvency.
    let entitlement = MAX_STAKE * TIER_C_RATIO;
    assert_eq!(entitlement, s.client.get_total_staked());
    do_override(&env, &s, &wa, &tx_hash(&env, 1), entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);
}

/// Bug 2 regression (eng review 2026-07-22): execute_override must not be
/// able to create a second, independently-payable claim on a wallet that
/// already has one in flight under a different tx_hash, the old code had
/// no equivalent to submit_claim's claim_active guard.
#[test]
fn override_blocks_second_claim_on_wallet_with_existing_claim() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    // Wallet already has a live claim (PendingTime, gate not met).
    submit_claim_signed(&env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &ENTITLEMENT, &TIER_C, &now_ts(&env));
    // A DIFFERENT tx_hash for the SAME wallet via override must be
    // refused: without the fix, this would create a second, independently
    // payable claim against the same forfeited stake.
    let hash2 = tx_hash(&env, 2);
    s.client
        .approve_override(&s.admin, &staker, &hash2, &ENTITLEMENT, &TIER_C);
    let result = s
        .client
        .try_approve_override(&s.co_signer, &staker, &hash2, &ENTITLEMENT, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::WalletHasDifferentActiveClaim)));
}

/// Bug 4 regression (eng review 2026-07-22): the daily-outflow-cap
/// subtraction must clamp, not panic, when the recomputed cap ends up
/// BELOW what's already been paid out that day, the scenario the old
/// `.max(0)`-after-subtract pattern could never actually protect against
/// (the subtraction itself panicked first, under this workspace's
/// `overflow-checks = true`). Forces exactly that: staker A collects
/// while utilization is low (cheap 500bps rate), then a same-day override
/// on a different wallet spikes utilization past 50%, shrinking the
/// recomputed cap below what A already collected today. A's next call
/// must fail with the graceful message, not a raw arithmetic panic.
#[test]
fn claim_stream_cap_shrinking_mid_day_fails_gracefully_not_via_panic() {
    // cap_base_a snapshot = anchor + A = 2 * MAX_STAKE. Utilisation at
    // submission is well under the first band edge, so the payout cap is the
    // first (widest) band's.
    let env = new_env();
    let s = setup(&env);
    let anchor = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor, &MAX_STAKE, &anchor_ben);

    let (staker_a, ben_a) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let cap_base_a = 2 * MAX_STAKE;
    let entitlement_a = bps_of(cap_base_a, BAND_1_UTILISATION_BPS / 2);
    assert!(entitlement_a * BPS_DENOMINATOR / cap_base_a < BAND_1_UTILISATION_BPS);
    advance_ledgers(&env, TIME_GATE_LEDGERS); // gate met
    let claim_a = submit_claim_signed(&env, &s, &s.oracle,
        &staker_a,
        &tx_hash(&env, 1),
        &entitlement_a, // well inside tier B's ceiling
        &TIER_B,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_a);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown and full vesting

    // Fully vested exceeds the payout cap, so this drains the day's cap in one
    // call.
    let first_cap = outflow_cap_oracle(cap_base_a, entitlement_a);
    assert!(first_cap < entitlement_a, "the cap must bind, not the vesting");
    let first = s.client.claim_stream(&claim_a, &ben_a);
    assert_eq!(first, first_cap);

    // Same day: a second wallet gets an overridden claim -- spikes
    // pool-wide total_allocated, and its own stake also raises
    // total_staked (which cap_base_a tracks via `max(total_staked_now,
    // snapshot)`), pushing utilization for A's own rate to the second band
    // edge. The new cap base is the anchor, A and B; the pool-wide allocation
    // is A's entitlement plus B's.
    let stake_b = bps_of(MAX_STAKE, 4_000);
    let staker_b = new_funded_address(&env, &s, stake_b);
    let ben_b = Address::generate(&env);
    s.client.stake(&staker_b, &stake_b, &ben_b);
    let entitlement_b = MAX_STAKE;
    do_override(&env, &s, &staker_b, &tx_hash(&env, 2), entitlement_b, TIER_B);
    let new_base = cap_base_a + stake_b;
    let new_allocated = entitlement_a + entitlement_b;
    assert!(new_allocated * BPS_DENOMINATOR / new_base >= BAND_2_UTILISATION_BPS);
    assert!(
        outflow_cap_oracle(new_base, new_allocated) < first,
        "the recomputed cap must already be below what A collected today"
    );

    // A's second call, same real day: the recomputed cap is now BELOW what
    // A already collected today. Old
    // code: `(cap - daily_outflow_so_far)` panics with a raw overflow
    // trap. Fixed code: saturating_sub clamps to 0, and the transfer
    // amount check produces the intended, graceful error instead.
    let result = s.client.try_claim_stream(&claim_a, &ben_a);
    assert_eq!(result, Err(Ok(PoolError::DailyOutflowCapReached)));
}

// -----------------------------------------------------------------------
// stake / withdraw / emergency_exit.
// -----------------------------------------------------------------------

/// A forfeited stake (withdrawn=true, amount kept live) passes the
/// `amount > 0 && !withdrawn` guard and is then refused by the lifetime ban
/// (founder rule 2026-09-23: once a claim is approved, never stake again).
/// Asserting the EXACT error still kills stake.rs's `&&`->`||` mutant: under
/// `||` this record would hit `AlreadyStaked` instead.
#[test]
fn restake_after_forfeiture_and_full_stream_is_refused_for_life() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    do_override(&env, &s, &staker, &hash, ENTITLEMENT, TIER_C);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    s.client.claim_stream(&claim_id, &ben); // completes the claim
    s.token_admin.mint(&staker, &MID_STAKE);
    let new_ben = Address::generate(&env);
    assert_eq!(
        s.client.try_stake(&staker, &MID_STAKE, &new_ben),
        Err(Ok(PoolError::AddressHasApprovedClaim))
    );
}

/// Staking to EXACTLY the pool cap must succeed (strict `>`).
/// Kills stake.rs:134:30 (>→>=).
#[test]
fn stake_filling_pool_cap_exactly_succeeds() {
    // A second stake can only be up to MAX_STAKE_BPS of the NEW cap, so the
    // new cap has to sit close above MID_STAKE rather than far above it.
    let env = new_env();
    let s = setup(&env);
    let (_w1, _b1) = staked_wallet(&env, &s);
    let second_stake = MID_STAKE / 1_000;
    let new_cap = MID_STAKE + second_stake;
    assert!(second_stake >= bps_of(new_cap, MIN_STAKE_BPS));
    assert!(second_stake <= max_stake(new_cap));
    s.client.set_pool_cap(&new_cap);
    let w2 = new_funded_address(&env, &s, second_stake);
    let b2 = Address::generate(&env);
    s.client.stake(&w2, &second_stake, &b2);
    assert_eq!(s.client.get_total_staked(), new_cap);
}

/// Overshooting the pool cap by ANY margin (not only hitting it exactly)
/// must panic.
/// Kills stake.rs:134:30 (>→==).
#[test]
fn stake_overshooting_pool_cap_panics() {
    // Same reasoning as stake_filling_pool_cap_exactly_succeeds, with the cap
    // a small margin below the total the second stake would reach. That total
    // is both > the cap and != the cap, which kills both the >= and == mutants.
    let env = new_env();
    let s = setup(&env);
    let (_w1, _b1) = staked_wallet(&env, &s);
    let second_stake = MID_STAKE / 1_000;
    let new_cap = MID_STAKE + second_stake - second_stake / 50;
    assert!(second_stake >= bps_of(new_cap, MIN_STAKE_BPS));
    assert!(second_stake <= max_stake(new_cap));
    s.client.set_pool_cap(&new_cap);
    let w2 = new_funded_address(&env, &s, second_stake);
    let b2 = Address::generate(&env);
    let result = s.client.try_stake(&w2, &second_stake, &b2);
    assert_eq!(result, Err(Ok(PoolError::PoolCapExceeded)));
}

/// Staker count increments by exactly 1 per stake.
/// Kills stake.rs:171:69 (+→*).
#[test]
fn total_stakers_increments_exactly() {
    let env = new_env();
    let s = setup(&env);
    let _ = staked_wallet(&env, &s);
    assert_eq!(s.client.get_total_stakers(), 1);
    let _ = staked_wallet(&env, &s);
    assert_eq!(s.client.get_total_stakers(), 2);
}

/// emergency_exit on a forfeited (not voluntarily-withdrawn) stake must
/// fail on the "no active stake" guard, reaching the claim_active check
/// instead would mean the || in the guard degraded to &&.
/// Kills stake.rs:276:27 (||→&&).
#[test]
fn emergency_exit_after_forfeiture_fails_on_active_stake_guard() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    do_override(&env, &s, &staker, &hash, ENTITLEMENT, TIER_C);
    let claim_id = claim_id_for(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    s.client.claim_stream(&claim_id, &ben); // Completed
    // Record: amount > 0, withdrawn=true → PoolError::NoActiveStake.
    let result = s.client.try_emergency_exit(&staker);
    assert_eq!(result, Err(Ok(PoolError::NoActiveStake)));
}

/// emergency_exit decrements total_staked by exactly the exiting amount.
/// Kills stake.rs:289:49 (−→+, −→/).
#[test]
fn emergency_exit_decrements_total_staked_exactly() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let _ = staked_wallet(&env, &s);
    assert_eq!(s.client.get_total_staked(), 2 * MID_STAKE);
    s.client.emergency_exit(&staker);
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
}

// -----------------------------------------------------------------------
// Penalty lock duration (types.rs).
// -----------------------------------------------------------------------

/// The false-positive-cancel penalty lock is 365 DAYS of ledgers, still
/// firmly locked after 2 days (a 365+17280-ledger mutant ≈ 1 day would
/// have expired).
/// Kills types.rs:40:43 (*→+ in PENALTY_LOCK_LEDGERS).
#[test]
fn penalty_lock_still_active_after_two_days() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    advance_ledgers(&env, TIME_GATE_LEDGERS);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    // CHANGED 2026-07-22: gate-met no longer auto-activates, the claim
    // must be explicitly approved before cancelling it exercises the
    // "was Active" penalty-lock branch this test is pinning.
    s.client.approve_claim(&claim_id);
    s.client.cancel_claim(&claim_id); // restores stake + penalty lock
    advance_days(&env, 2);
    let result = s.client.try_withdraw(&staker, &ben);
    assert_eq!(result, Err(Ok(PoolError::PenaltyLockActive)));
}

// -----------------------------------------------------------------------
// Instance-TTL bumps (storage.rs).
// -----------------------------------------------------------------------

/// Every state-touching entrypoint bumps the instance TTL to exactly
/// BUMP_TO (120 days of ledgers), and a later call re-bumps once the
/// remaining TTL falls below BUMP_THRESHOLD (30 days), the second phase
/// distinguishes the real threshold (30 days of ledgers) from a mutated
/// one (30 + one day of ledgers).
/// Kills storage.rs:23:32 (*→+, *→/) and 234:5 (bump_instance_ttl→()).
#[test]
fn instance_ttl_bumped_to_exact_target_and_rebumped_below_threshold() {
    use soroban_sdk::testutils::storage::Instance;
    let env = new_env();
    let s = setup(&env);
    let contract_id = s.client.address.clone();
    let _ = staked_wallet(&env, &s); // any bumping entrypoint
    let bump_to = 120 * LEDGERS_PER_DAY;
    let ttl_after_stake = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
    assert_eq!(ttl_after_stake, bump_to);
    // Age the instance so the remaining TTL sits BETWEEN the mutated threshold
    // and the real one: the real code re-bumps, the mutant would not.
    let real_threshold = 30 * LEDGERS_PER_DAY;
    let mutated_threshold = 30 + LEDGERS_PER_DAY;
    let remaining_ttl = (real_threshold + mutated_threshold) / 2;
    assert!(mutated_threshold < remaining_ttl && remaining_ttl < real_threshold);
    advance_ledgers(&env, bump_to - remaining_ttl);
    let _ = staked_wallet(&env, &s);
    let ttl_after_second = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
    assert_eq!(ttl_after_second, bump_to);
}
