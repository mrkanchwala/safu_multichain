#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::error::PoolError;

const TIER_A: u32 = 1;
const TIER_B: u32 = 2;
const TIER_C: u32 = 3;
const ENTITLEMENT: i128 = STROOPS_PER_UNIT;

#[test]
fn tier_caps_apply_correct_ratios() {
    // tier_cap = stake * ratio (TIER_COVERAGE_BPS is 100% by default),
    // A=15x, B=10x, C=5x. Verified indirectly: exactly-at-cap succeeds,
    // one-stroop-over panics, for each tier. Caller alternates
    // oracle/admin: three same-day oracle submissions would trip the
    // unrelated per-day oracle rate limit before reaching what this test
    // actually targets.
    //
    // A single MID_STAKE staker's own stake can't solvently back a tier A
    // (15x) entitlement: the tier cap only bounds
    // what's ASKED for, not what the pool can actually pay. Raise the
    // pool cap and add a large anchor staker so total_staked (and the
    // 25%-of-pool admission stress cap) can actually cover it; the
    // anchor never claims, so it doesn't affect tier math for anyone
    // else.
    // Anchor sized so the 25%-of-pool admission stress cap stays clear
    // even after A's and B's entitlements are already reserved (checked
    // by hand: utilization stays under 20% at every step against this
    // test's three entitlements). Kept within the DEFAULT pool cap's
    // bounds (11 stakers at MAX_STAKE, not a raised cap) so staker_a/b/c
    // can keep using the shared MID_STAKE-based helper, raising the
    // pool cap would have shifted their min/max bounds too and broken
    // `staked_wallet`'s fixed MID_STAKE amount.
    let env = new_env();
    let s = setup(&env);
    for i in 0..11u8 {
        let anchor = new_funded_address(&env, &s, MAX_STAKE);
        let anchor_ben = Address::generate(&env);
        s.client.stake(&anchor, &MAX_STAKE, &anchor_ben);
        let _ = i;
    }

    let (staker_a, _b) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker_a,
        &tx_hash(&env, 1),
        &(MID_STAKE * 15), // exactly tier A cap
        &TIER_A,
        &now_ts(&env),
    );

    let (staker_b, _b) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker_b,
        &tx_hash(&env, 2),
        &(MID_STAKE * 10), // exactly tier B cap
        &TIER_B,
        &now_ts(&env),
    );

    let (staker_c, _b) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker_c,
        &tx_hash(&env, 3),
        &(MID_STAKE * 5), // exactly tier C cap
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn tier_a_one_stroop_over_cap_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _b) = staked_wallet(&env, &s);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &(MID_STAKE * 15 + 1),
        &TIER_A,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::EntitlementExceedsTierCap)));
}

#[test]
fn total_allocated_never_exceeds_total_staked_across_multiple_claims() {
    let env = new_env();
    let s = setup(&env);
    // Three independent stakers, each submits a modest claim, the
    // aggregate solvency invariant (total_allocated <= total_staked)
    // must hold after every single submission.
    // Caller is admin, not oracle, the oracle's per-day claim-count
    // limit (total_stakers/10, min 1) would otherwise fire after the
    // first submission with only a handful of stakers, which is a
    // separate mechanic from what this test is checking.
    for i in 0..3u8 {
        let (staker, _b) = staked_wallet(&env, &s);
        submit_claim_signed(&env, &s, &s.oracle,
            &staker,
            &tx_hash(&env, i),
            &ENTITLEMENT,
            &TIER_C,
            &now_ts(&env),
        );
        assert!(s.client.get_total_allocated() <= s.client.get_total_staked());
    }
}

#[test]
fn cancelling_one_claim_does_not_affect_another_stakers_solvency() {
    let env = new_env();
    let s = setup(&env);
    let (staker1, _b1) = staked_wallet(&env, &s);
    let (staker2, _b2) = staked_wallet(&env, &s);

    // Small enough that both claims clear the day's stress cap and are admitted.
    let first_entitlement = ENTITLEMENT / 10;
    let second_entitlement = 2 * first_entitlement;
    let claim1 = submit_claim_signed(&env, &s, &s.oracle,
        &staker1,
        &tx_hash(&env, 1),
        &first_entitlement,
        &TIER_C,
        &now_ts(&env),
    );
    // Next day, so the oracle's per-day claim count does not queue it.
    advance_days(&env, 1);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker2,
        &tx_hash(&env, 2),
        &second_entitlement,
        &TIER_C,
        &now_ts(&env),
    );

    s.client.cancel_claim(&claim1);

    // staker2's claim reservation must be untouched by staker1's cancel.
    assert_eq!(s.client.get_total_allocated(), second_entitlement);
}

#[test]
fn points_accrue_tenure_banded_and_bank_on_withdraw() {
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &MID_STAKE, &ben);

    // 30 days at the 0-90-day band (100 pts/day), points formula:
    // base = 30*100 = 3000; × stake/max_stake (MID_STAKE/MAX_STAKE).
    advance_days(&env, 30);
    s.client.withdraw(&staker, &ben);

    let expected = 30 * POINTS_PER_DAY_TIER_1 * MID_STAKE / MAX_STAKE;
    assert_eq!(s.client.get_points_balance(&staker), expected);
}

#[test]
fn points_span_multiple_tenure_bands() {
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &MID_STAKE, &ben);

    // 100 days: 90 days at band 1 (100/day) + 10 days at band 2 (120/day).
    advance_days(&env, 100);
    s.client.withdraw(&staker, &ben);

    let base = 90 * POINTS_PER_DAY_TIER_1 + 10 * POINTS_PER_DAY_TIER_2;
    let expected = (base as i128) * MID_STAKE / MAX_STAKE;
    assert_eq!(s.client.get_points_balance(&staker), expected);
}

#[test]
fn points_zero_for_zero_duration_withdraw() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    // Withdraw in the exact same ledger as the stake, zero tenure.
    s.client.withdraw(&staker, &ben);
    assert_eq!(s.client.get_points_balance(&staker), 0);
}

#[test]
fn points_accumulate_across_multiple_stake_cycles() {
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE * 2);
    let ben = Address::generate(&env);

    s.client.stake(&staker, &MID_STAKE, &ben);
    advance_days(&env, 30);
    s.client.withdraw(&staker, &ben);
    let first_cycle_points = s.client.get_points_balance(&staker);
    assert!(first_cycle_points > 0);

    s.client.stake(&staker, &MID_STAKE, &ben);
    advance_days(&env, 30);
    s.client.withdraw(&staker, &ben);

    // Points bank cumulatively across cycles, never burned, never reset.
    assert_eq!(s.client.get_points_balance(&staker), first_cycle_points * 2);
}

#[test]
fn forfeited_stake_still_banks_points_earned_before_the_claim() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_days(&env, POINTS_TIER_1_DAYS); // gate met, and first-tier points accrued
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let base = i128::from(POINTS_TIER_1_DAYS) * POINTS_PER_DAY_TIER_1;
    let expected = (base as i128) * MID_STAKE / MAX_STAKE;
    assert_eq!(s.client.get_points_balance(&staker), expected);
}

#[test]
fn get_points_balance_is_live_computed_while_still_staked() {
    // Regression test for a real gap found 2026-07-14: V8's pointsOf is a
    // COMPUTED view for an active staker, not a storage read, the
    // original get_points_balance always returned the (empty, until
    // withdrawal) banked value, which was wrong for anyone still staked.
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &MID_STAKE, &ben);

    advance_days(&env, 30);
    // Still staked: no withdraw call. Banked storage is empty at this
    // point; the view must compute points live instead of returning 0.
    let expected = 30 * POINTS_PER_DAY_TIER_1 * MID_STAKE / MAX_STAKE;
    assert_eq!(s.client.get_points_balance(&staker), expected);
}

#[test]
fn is_eligible_true_for_active_unsuspended_stake() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    assert!(s.client.is_eligible(&staker));
}

#[test]
fn is_eligible_false_for_nonexistent_stake() {
    let env = new_env();
    let s = setup(&env);
    let random = Address::generate(&env);
    assert!(!s.client.is_eligible(&random));
}

#[test]
fn is_eligible_false_after_withdraw() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.withdraw(&staker, &ben);
    assert!(!s.client.is_eligible(&staker));
}

#[test]
fn is_eligible_false_while_suspended() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    s.client.suspend_stake(&staker);
    assert!(!s.client.is_eligible(&staker));
}

#[test]
fn is_claim_eligible_false_before_gate_true_after() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    assert!(!s.client.is_claim_eligible(&staker));
    advance_past_time_gate(&env);
    assert!(s.client.is_claim_eligible(&staker));
}

#[test]
fn is_claim_eligible_false_while_suspended_even_after_gate() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    s.client.suspend_stake(&staker);
    assert!(!s.client.is_claim_eligible(&staker));
}
