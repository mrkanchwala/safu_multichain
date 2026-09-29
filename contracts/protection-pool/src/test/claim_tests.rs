#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, IntoVal};

use super::common::*;
use crate::error::PoolError;
use crate::types::{
    ClaimStatus, APPROVE_WINDOW_LEDGERS, BPS_DENOMINATOR, CLAIM_WINDOW_SECONDS, COLLECTION_INACTIVITY_LEDGERS,
    COOLDOWN_LEDGERS, PENALTY_LOCK_LEDGERS, TIER_C_RATIO, TIME_GATE_LEDGERS, VESTING_LEDGERS,
};

const ENTITLEMENT: i128 = STROOPS_PER_UNIT;
const TIER_C: u32 = 3;

/// An entitlement small enough that a fully vested claim clears the day's
/// payout cap in a single call, so a test can reach `Completed`. Sized from
/// the stake so it holds whatever the stake bounds are.
const SINGLE_CALL_ENTITLEMENT: i128 = MID_STAKE / 25;

/// The claim window (in ledgers) that `submit_claim` accepts a hack timestamp inside.
const CLAIM_WINDOW_LEDGERS: u32 = (CLAIM_WINDOW_SECONDS / SECONDS_PER_LEDGER) as u32;

/// Utilisation, against the pool the claim was made in, that sits inside the
/// second outflow band, and one just above the first band edge.
const IN_SECOND_BAND_UTILISATION_BPS: i128 = 2_400;
const JUST_ABOVE_FIRST_BAND_UTILISATION_BPS: i128 = 2_050;


// -----------------------------------------------------------------------
// submit_claim: happy paths
// -----------------------------------------------------------------------

#[test]
fn submit_claim_by_oracle_pending_when_gate_not_met() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::PendingTime);
    assert_eq!(claim.entitlement, ENTITLEMENT);
    // Stake not forfeited yet: total_staked unchanged.
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
}

#[test]
fn submit_claim_awaiting_approval_when_gate_already_met() {
    // CHANGED 2026-07-22: meeting the gate at submission time no longer
    // auto-activates (forfeits/burns/starts cooldown), it lands in
    // AwaitingApproval with a deadline set, and the staker must actively
    // call approve_claim (Rule A) before anything is forfeited.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::AwaitingApproval);
    assert!(claim.approve_deadline_ledger > 0);
    // Nothing forfeited yet: total_staked/stakers unchanged.
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
    assert_eq!(s.client.get_total_stakers(), 1);
}

#[test]
fn approve_claim_forfeits_and_activates() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
    assert_eq!(s.client.get_total_staked(), 0);
    assert_eq!(s.client.get_total_stakers(), 0);
    // Rule B's clock anchors at cooldown end, not the approval ledger.
    assert_eq!(claim.last_collected_ledger, claim.cooldown_ends_ledger);
}

#[test]
fn approve_claim_burns_entire_lifetime_points_balance() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    advance_days(&env, POINTS_TIER_1_DAYS);
    // First cycle: withdraw before ever claiming, banking points into the
    // wallet's lifetime balance.
    s.client.withdraw(&staker, &ben);
    let banked_from_cycle_1 = s.client.get_points_balance(&staker);
    assert!(banked_from_cycle_1 > 0);

    // Second cycle: stake again, get hacked, approve, should burn BOTH
    // this cycle's points AND the banked balance from cycle 1. withdraw()
    // sent the first cycle's principal to `ben` (the beneficiary), not
    // back to `staker`: needs fresh funding to stake again.
    s.token_admin.mint(&staker, &MID_STAKE);
    s.client.stake(&staker, &MID_STAKE, &ben);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);
    assert_eq!(s.client.get_points_balance(&staker), 0);
}

#[test]
fn approve_claim_before_gate_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    // Still PendingTime: gate not met, never transitioned to AwaitingApproval.
    let result = s.client.try_approve_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ClaimNotAwaitingApproval)));
}

#[test]
fn approve_claim_after_100_days_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + 1);
    let result = s.client.try_approve_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ApprovalWindowExpired)));
}

#[test]
fn approve_claim_while_suspended_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.suspend_stake(&staker);
    let result = s.client.try_approve_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::StakeSuspended)));
}

// -----------------------------------------------------------------------
// expire_pending_approval: Rule A sweep
// -----------------------------------------------------------------------

#[test]
fn expire_pending_approval_releases_reservation() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + 1);
    s.client.expire_pending_approval(&claim_id);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Expired);
    assert_eq!(s.client.get_total_allocated(), 0);
    // Nothing was ever forfeited: stake is untouched and withdrawable.
    s.client.withdraw(&staker, &ben);
}

/// Audit finding 2026-07-22 (adversarial /audit-chain re-review): the
/// suspend upgrade was dead code for any already-approved claim until
/// admin.rs's `suspend_stake` guard was corrected to allow suspending a
/// forfeited-but-still-active stake, as well as a pre-forfeiture one. This
/// proves the positive path actually works end-to-end: suspend mid-
/// streaming genuinely blocks the next collection.
#[test]
fn suspend_during_active_streaming_blocks_claim_stream() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 4); // cooldown, then a quarter of vesting
    s.client.claim_stream(&claim_id, &ben); // proves streaming works first
    s.client.suspend_stake(&staker); // now reachable post-forfeiture
    advance_ledgers(&env, VESTING_LEDGERS / 4);
    let result = s.client.try_claim_stream(&claim_id, &ben); // blocked while suspended
    assert_eq!(result, Err(Ok(PoolError::StakeSuspended)));
}

/// Audit finding 2026-07-22 (adversarial re-review, /audit-chain): a
/// suspended staker's Rule A clock must not be sweepable while they're
/// still frozen out of acting, otherwise staying suspended past the
/// deadline (never explicitly unsuspended) loses them the reservation
/// regardless, defeating blocker #1's fairness fix.
#[test]
fn expire_pending_approval_blocked_while_suspended() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.suspend_stake(&staker);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + 1); // deadline genuinely passed
    let result = s.client.try_expire_pending_approval(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::StakeSuspended)));
}

#[test]
fn expire_pending_approval_before_deadline_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS - 1); // one ledger inside the window
    let result = s.client.try_expire_pending_approval(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ApprovalWindowNotExpired)));
}

// -----------------------------------------------------------------------
// expire_stale_claim: Rule B sweep
// -----------------------------------------------------------------------

#[test]
fn expire_stale_claim_after_100_days_inactivity_releases_remainder() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 4); // cooldown, then a quarter of vesting
    let transferred = s.client.claim_stream(&claim_id, &ben);
    assert!(transferred > 0);

    advance_ledgers(&env, COLLECTION_INACTIVITY_LEDGERS + 1); // past the inactivity window with zero further activity
    s.client.expire_stale_claim(&claim_id);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Expired);
    assert_eq!(s.client.get_total_allocated(), 0);
}

#[test]
fn expire_stale_claim_zero_streamed_case() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, _ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    // Never called claim_stream even once.
    advance_ledgers(&env, COOLDOWN_LEDGERS + COLLECTION_INACTIVITY_LEDGERS + 1);
    s.client.expire_stale_claim(&claim_id);
    assert_eq!(s.client.get_total_allocated(), 0);
}

/// Audit finding 2026-07-22: same fairness gap as
/// expire_pending_approval_blocked_while_suspended, but for Rule B.
#[test]
fn expire_stale_claim_blocked_while_suspended() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 4); // cooldown, then a quarter of vesting
    s.client.claim_stream(&claim_id, &ben);
    s.client.suspend_stake(&staker);
    advance_ledgers(&env, COLLECTION_INACTIVITY_LEDGERS + 1); // genuinely stale now
    let result = s.client.try_expire_stale_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::StakeSuspended)));
}

#[test]
fn expire_stale_claim_before_100_days_panics() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 4); // cooldown, then a quarter of vesting
    s.client.claim_stream(&claim_id, &ben);
    advance_ledgers(&env, COLLECTION_INACTIVITY_LEDGERS - 1); // resets from the collection above, not yet stale
    let result = s.client.try_expire_stale_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ClaimNotStale)));
}

#[test]
fn claim_stream_resets_rule_b_clock() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS / 4); // cooldown, then a quarter of vesting
    s.client.claim_stream(&claim_id, &ben);
    // Without the reset, this wait plus the prior gap would trip Rule B,
    // collecting again proves the clock genuinely moved forward.
    advance_ledgers(&env, COLLECTION_INACTIVITY_LEDGERS - 1);
    let transferred = s.client.claim_stream(&claim_id, &ben);
    assert!(transferred > 0);
}

#[test]
fn submit_claim_by_admin_also_works() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn submit_claim_banks_points_on_immediate_activation() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_days(&env, POINTS_TIER_1_DAYS);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert!(s.client.get_points_balance(&staker) > 0);
}

#[test]
fn submit_claim_reserves_total_allocated() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT);
}

// -----------------------------------------------------------------------
// submit_claim: validation panics
// -----------------------------------------------------------------------

#[test]
fn submit_claim_wrong_caller_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let random = Address::generate(&env);
    let result = try_submit_claim_signed(&env, &s, &random,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::CallerNotOracleOrAdmin)));
}

#[test]
fn submit_claim_zero_entitlement_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &0,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::EntitlementNotPositive)));
}

#[test]
fn submit_claim_invalid_tier_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &4,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::InvalidTier)));
}

#[test]
fn submit_claim_no_stake_panics() {
    let env = new_env();
    let s = setup(&env);
    let random = Address::generate(&env);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &random,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::NoStake)));
}

#[test]
fn submit_claim_after_withdraw_panics() {
    // Reaches PoolError::NoActiveStake (amount<=0), not AlreadyWithdrawn.
    // Same check-ordering note as elsewhere in this suite.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.withdraw(&staker, &ben);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::NoActiveStake)));
}

#[test]
fn submit_claim_twice_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 2),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::ClaimAlreadyActiveForStake)));
}

#[test]
fn submit_claim_exceeds_tier_cap_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    // Tier C cap = stake * TIER_C_RATIO; ask for one unit more.
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &(MID_STAKE * TIER_C_RATIO + 1),
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::EntitlementExceedsTierCap)));
}

#[test]
fn submit_claim_future_hack_timestamp_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &(now_ts(&env) + 1),
    );
    assert_eq!(result, Err(Ok(PoolError::HackTimestampInFuture)));
}

#[test]
fn submit_claim_hack_before_stake_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &(now_ts(&env) - 1),
    );
    assert_eq!(result, Err(Ok(PoolError::HackPredatesStake)));
}

#[test]
fn submit_claim_outside_claim_window_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hack_ts = now_ts(&env);
    advance_ledgers(&env, CLAIM_WINDOW_LEDGERS + 1); // just past the claim window
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &hack_ts,
    );
    assert_eq!(result, Err(Ok(PoolError::ClaimWindowExpired)));
}

#[test]
fn submit_claim_at_exactly_claim_window_boundary_succeeds() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hack_ts = now_ts(&env);
    advance_ledgers(&env, CLAIM_WINDOW_LEDGERS); // exactly at the boundary, still valid
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &hack_ts,
    );
}

#[test]
fn submit_claim_insolvent_queues_instead_of_rejecting() {
    // T3 (2026-08-24): Insolvent no longer hard-rejects, it queues
    // (ClaimStatus::Reserved) so the genuine incident is re-triable once
    // total_staked/total_allocated state changes, instead of vanishing.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    // Only MID_STAKE actually backing the pool; ask for more than that even
    // though it's under the tier C cap (MID_STAKE * TIER_C_RATIO).
    let entitlement = MID_STAKE + 1;
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &entitlement,
        &TIER_C,
        &now_ts(&env),
    );
    let claim_id = result.unwrap().unwrap();
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.entitlement, entitlement);
    // Queuing must NOT touch capacity accounting, that's the entire point:
    // it wasn't actually admitted.
    assert_eq!(s.client.get_total_allocated(), 0);
    // Second submission for the same wallet, different claim, must be
    // rejected while one is Reserved.
    let blocked = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 2),
        &1,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(blocked, Err(Ok(PoolError::ClaimAlreadyQueued)));
}

#[test]
fn submit_claim_exceeds_stress_cap_queues_instead_of_rejecting() {
    // T3 (2026-08-24): same as the Insolvent case above, DailyStressCapExceeded
    // now queues instead of hard-rejecting.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    // Ask for one unit more than the stress cap at zero utilisation, which is
    // still within solvency and tier bounds.
    let entitlement = expected_stress_cap(&s) + 1;
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &entitlement,
        &TIER_C,
        &now_ts(&env),
    );
    let claim_id = result.unwrap().unwrap();
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.entitlement, entitlement);
}

#[test]
fn submit_claim_oracle_rate_limit_queues() {
    // T3 (2026-08-24, second pass): the oracle's own daily submission-count
    // throttle now QUEUES rather than hard-rejecting, same as the capacity
    // limits. The limit still binds, the second claim is not admitted, but
    // a genuine hack is no longer lost because the oracle already filed its
    // quota that day. This matters most in a mass incident, which is exactly
    // when `total_stakers / 10` is most likely to bite.
    let env = new_env();
    let s = setup(&env);
    // total_stakers/10 max(1) == 1 with a single staker, the SECOND
    // oracle-submitted claim same day must not be ADMITTED even though it's
    // a different wallet.
    let (staker1, _b1) = staked_wallet(&env, &s);
    let (staker2, _b2) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker1,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let allocated_after_first = s.client.get_total_allocated();

    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker2,
        &tx_hash(&env, 2),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let claim_id = result.unwrap().unwrap();
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.entitlement, ENTITLEMENT);
    // The throttle still does its job: nothing extra was admitted, so no
    // capacity was consumed by the queued claim.
    assert_eq!(s.client.get_total_allocated(), allocated_after_first);
    assert!(s.client.get_stake(&staker2).unwrap().reserved_claim_id.is_some());
}

#[test]
fn submit_claim_admin_not_subject_to_oracle_rate_limit() {
    let env = new_env();
    let s = setup(&env);
    let (staker1, _b1) = staked_wallet(&env, &s);
    let (staker2, _b2) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker1,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    // Admin call for a second wallet, same day, not gated by the
    // oracle-only rate limit.
    submit_claim_signed(&env, &s, &s.oracle,
        &staker2,
        &tx_hash(&env, 2),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn submit_claim_oracle_limit_resets_next_day() {
    let env = new_env();
    let s = setup(&env);
    let (staker1, _b1) = staked_wallet(&env, &s);
    let (staker2, _b2) = staked_wallet(&env, &s);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker1,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_days(&env, 1);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker2,
        &tx_hash(&env, 2),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn submit_claim_duplicate_wallet_tx_hash_after_cancel_panics() {
    // claim_active alone would block a same-wallet resubmit with
    // ClaimAlreadyActiveForStake: that flag resets on cancel_claim, so
    // this test specifically isolates the SEPARATE claim-id-existence
    // guard: a cancelled claim's id is permanently retired, never reused.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    let claim_id =
        submit_claim_signed(&env, &s, &s.oracle, &staker, &hash, &ENTITLEMENT, &TIER_C, &now_ts(&env));
    s.client.cancel_claim(&claim_id);
    // Advance a day so the oracle's daily claim-count limit (unrelated to
    // what this test targets) doesn't fire first and mask the real
    // assertion.
    advance_days(&env, 1);
    let result = try_submit_claim_signed(&env, &s, &s.oracle, &staker, &hash, &ENTITLEMENT, &TIER_C, &now_ts(&env));
    assert_eq!(result, Err(Ok(PoolError::ClaimAlreadyExists)));
}

/// CHANGED 2026-08-17 (7a audit, Finding 5): was
/// `#[should_panic(expected = "SAFU: paused")]`. `require_not_paused` now
/// returns the typed `PoolError::Paused` rather than a bare `panic!`, so this
/// asserts the specific error code, strictly stronger than matching a panic
/// substring, and consistent with how every other rejection in this file is
/// tested.
#[test]
fn submit_claim_blocked_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    s.client.pause();
    let result = try_submit_claim_signed(
        &env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::Paused)));
}

// -----------------------------------------------------------------------
// unlock_pending_claim
// -----------------------------------------------------------------------

#[test]
fn unlock_pending_claim_moves_to_awaiting_approval_after_gate() {
    // CHANGED 2026-07-22: unlock_pending_claim no longer activates
    // directly: like submit_claim's gate-met branch, it now lands in
    // AwaitingApproval, still gated on the staker's own approve_claim.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_past_time_gate(&env);
    s.client.unlock_pending_claim(&claim_id);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::AwaitingApproval);
    assert!(claim.approve_deadline_ledger > 0);
    assert_eq!(s.client.get_total_staked(), MID_STAKE);

    // And approving from here works exactly like the submit_claim path.
    s.client.approve_claim(&claim_id);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
    assert_eq!(s.client.get_total_staked(), 0);
}

#[test]
fn unlock_pending_claim_before_gate_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_ledgers(&env, TIME_GATE_LEDGERS - 1); // one ledger short of the gate
    let result = s.client.try_unlock_pending_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::TimeGateNotMet)));
}

#[test]
fn unlock_pending_claim_already_active_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    let result = s.client.try_unlock_pending_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ClaimNotPending)));
}

#[test]
fn unlock_pending_claim_nonexistent_panics() {
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_unlock_pending_claim(&tx_hash(&env, 99));
    assert_eq!(result, Err(Ok(PoolError::NoSuchClaim)));
}

// -----------------------------------------------------------------------
// claim_stream
// -----------------------------------------------------------------------

/// CHANGED 2026-07-22 (points burn-on-claim mechanism): meeting the gate
/// no longer auto-activates: it lands in AwaitingApproval, and the
/// staker must call `approve_claim` themselves (Rule A) before the stake
/// forfeits and cooldown/vesting starts. Added that call here so every
/// test using this helper still gets a genuinely Active claim, same as
/// before the mechanism change.
fn active_claim_with_entitlement(
    env: &soroban_sdk::Env,
    s: &Setup<'_>,
    entitlement: i128,
) -> (Address, Address, soroban_sdk::BytesN<32>) {
    let (staker, ben) = staked_wallet(env, s);
    advance_past_time_gate(env); // gate met, lands in AwaitingApproval on submit
    let claim_id = submit_claim_signed(env, s, &s.oracle,
        &staker,
        &tx_hash(env, 1),
        &entitlement,
        &TIER_C,
        &now_ts(env),
    );
    s.client.approve_claim(&claim_id);
    (staker, ben, claim_id)
}

/// Mutation-testing gap fix (2026-07-22 re-run). Kills claim.rs:443
/// (`>`->`>=`), at exactly the deadline ledger the window has NOT yet
/// expired (only strictly-after should panic); the mutant would
/// incorrectly reject a valid on-time approval.
#[test]
fn approve_claim_at_exactly_deadline_succeeds() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS); // exactly the window later
    s.client.approve_claim(&claim_id); // must NOT panic
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
}

/// Kills claim.rs:208 x2 (`+`->`-`, `+`->`*` in `lifetime_balance = banked
/// + points`). Storage is unconditionally zeroed on burn regardless of
/// this sum's correctness, so the only observable surface is the
/// `ClaimApproved` event's `points_burned` field: read directly rather
/// than via a storage getter. Uses a withdraw-then-restake cycle so BOTH
/// `banked` (from cycle 1) and `points` (cycle 2, freshly accrued) are
/// nonzero and equal, making a subtraction (0) or a multiplication (the
/// square of one cycle) trivially distinguishable from the correct sum.
#[test]
fn approve_claim_burns_prior_banked_plus_new_points_exactly() {
    let env = new_env();
    let s = setup(&env);
    let cycle_points = i128::from(POINTS_TIER_1_DAYS) * POINTS_PER_DAY_TIER_1 * MID_STAKE / MAX_STAKE;
    let beneficiary = Address::generate(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE);
    s.client.stake(&staker, &MID_STAKE, &beneficiary);
    advance_days(&env, POINTS_TIER_1_DAYS); // cycle 1: first-tier days accrued
    s.client.withdraw(&staker, &beneficiary); // banks that cycle's points, amount -> 0

    s.token_admin.mint(&staker, &MID_STAKE); // withdraw paid the beneficiary, refund staker
    s.client.stake(&staker, &MID_STAKE, &beneficiary); // fresh record
    advance_days(&env, POINTS_TIER_1_DAYS); // cycle 2: the same days again -> the same points

    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);

    let events = env.events().all();
    let event = events.events().last().unwrap();
    let data_scval = match &event.body {
        soroban_sdk::xdr::ContractEventBody::V0(v0) => &v0.data,
    };
    let xdr_bytes = soroban_sdk::xdr::WriteXdr::to_xdr(data_scval, soroban_sdk::xdr::Limits::none())
        .unwrap();
    let bytes = soroban_sdk::Bytes::from_slice(&env, &xdr_bytes);
    let data_val: soroban_sdk::Val = soroban_sdk::xdr::FromXdr::from_xdr(&env, &bytes).unwrap();
    let data_map: soroban_sdk::Map<soroban_sdk::Symbol, soroban_sdk::Val> =
        data_val.into_val(&env);
    let points_burned: i128 = data_map
        .get(soroban_sdk::Symbol::new(&env, "points_burned"))
        .unwrap()
        .into_val(&env);
    // Each cycle earns the same points, so the burn is exactly twice one cycle: the
    // sum, and not the 0 a subtraction gives or the square a multiplication gives.
    assert_eq!(points_burned, 2 * cycle_points);
    assert_ne!(points_burned, 0);
    assert_ne!(points_burned, cycle_points * cycle_points);
}

#[test]
fn claim_stream_before_cooldown_panics() {
    let env = new_env();
    let s = setup(&env);
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, ENTITLEMENT);
    let result = s.client.try_claim_stream(&claim_id, &ben);
    assert_eq!(result, Err(Ok(PoolError::CooldownNotPassed)));
}

#[test]
fn claim_stream_partial_vesting_amount() {
    // Vested fraction derived from VESTING_LEDGERS itself instead of a
    // hand-picked day count.
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS); // cooldown passes
    let elapsed = VESTING_LEDGERS / 4;
    advance_ledgers(&env, elapsed); // 1/4 of vesting elapsed
    let transferred = s.client.claim_stream(&claim_id, &ben);
    assert_eq!(transferred, entitlement * elapsed as i128 / VESTING_LEDGERS as i128);
}

#[test]
fn claim_stream_full_after_vesting_completes() {
    // Recalibrated 2026-09-19: MID_STAKE is now 55M (was 100M), so the
    // outflow cap at the default <20%-utilization band (500bps of 55M =
    // 2.75M) is now below 4.5M -- the old entitlement would get capped
    // mid-stream instead of completing in one call. 2M clears both the
    // utilization band (2M/55M ~3.6% < 20%) and the cap itself.
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested
    let transferred = s.client.claim_stream(&claim_id, &ben);
    assert_eq!(transferred, entitlement);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Completed);
}

#[test]
fn claim_stream_after_completion_panics() {
    // Recalibrated 2026-09-19 -- same reasoning as
    // claim_stream_full_after_vesting_completes: needs an entitlement that
    // clears the outflow cap in one call so the claim actually reaches
    // Completed before the second call is made.
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested
    s.client.claim_stream(&claim_id, &ben);
    let result = s.client.try_claim_stream(&claim_id, &ben);
    assert_eq!(result, Err(Ok(PoolError::ClaimNotActive)));
}

#[test]
fn claim_stream_wrong_beneficiary_panics() {
    let env = new_env();
    let s = setup(&env);
    let (_staker, _ben, claim_id) = active_claim_with_entitlement(&env, &s, ENTITLEMENT);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested
    let wrong = Address::generate(&env);
    let result = s.client.try_claim_stream(&claim_id, &wrong);
    assert_eq!(result, Err(Ok(PoolError::WrongBeneficiary)));
}

#[test]
fn claim_stream_nonexistent_claim_panics() {
    let env = new_env();
    let s = setup(&env);
    let ben = Address::generate(&env);
    let result = s.client.try_claim_stream(&tx_hash(&env, 99), &ben);
    assert_eq!(result, Err(Ok(PoolError::NoSuchClaim)));
}

#[test]
fn claim_stream_daily_outflow_cap_limits_large_payout() {
    let env = new_env();
    let s = setup(&env);
    // A second, non-claiming staker inflates the pool so the claim's
    // entitlement clears the tier/stress/solvency checks while still
    // being big enough to trip the 3%-of-base daily outflow cap.
    let anchor_staker = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor_staker, &MAX_STAKE, &anchor_ben);

    // cap_base = max(total_staked_now, snapshot) = MAX_STAKE + MID_STAKE,
    // snapshotted before this claim's forfeiture. The entitlement has to clear
    // admission (it must not exceed the stress cap or it queues and
    // approve_claim fails with ClaimNotAwaitingApproval) while keeping
    // utilisation inside the second outflow band.
    let cap_base = MAX_STAKE + MID_STAKE;
    let entitlement = bps_of(cap_base, IN_SECOND_BAND_UTILISATION_BPS);
    assert!(entitlement <= stress_cap_oracle(cap_base, 0), "must clear admission");
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested

    let transferred = s.client.claim_stream(&claim_id, &ben);
    let expected_cap = outflow_cap_oracle(cap_base, entitlement);
    assert!(expected_cap < entitlement, "the cap, not the vesting, must limit this call");
    assert_eq!(transferred, expected_cap);
}

#[test]
fn claim_stream_second_call_same_day_hits_cap() {
    // cap_base is the anchor plus the staker. The entitlement clears admission
    // and keeps POST-payout utilisation still inside the second outflow band,
    // so the same-day rate doesn't improve and the second call has genuinely
    // nothing left (see claim_stream_next_day_cap_resets for the contrasting
    // scenario where the rate DOES improve).
    let env = new_env();
    let s = setup(&env);
    let anchor_staker = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor_staker, &MAX_STAKE, &anchor_ben);

    let cap_base = MAX_STAKE + MID_STAKE;
    let entitlement = bps_of(cap_base, IN_SECOND_BAND_UTILISATION_BPS);
    assert!(entitlement <= stress_cap_oracle(cap_base, 0), "must clear admission");
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested

    let first = s.client.claim_stream(&claim_id, &ben); // drains the day's cap
    assert_eq!(first, outflow_cap_oracle(cap_base, entitlement));
    // Utilisation after the payout is still in the second band, so the rate has
    // not improved.
    assert!((entitlement - first) * BPS_DENOMINATOR / cap_base >= BAND_1_UTILISATION_BPS);
    let result = s.client.try_claim_stream(&claim_id, &ben); // same day, same rate, nothing left
    assert_eq!(result, Err(Ok(PoolError::DailyOutflowCapReached)));
}

#[test]
fn claim_stream_next_day_cap_resets() {
    // cap_base is the anchor plus the staker. The entitlement keeps pre-payout
    // utilisation just above the first band edge, so the first day's rate is
    // the second band's. After that payout, allocated drops and utilisation
    // falls below the edge, so the rate correctly improves on the new day and
    // the remaining claimable amount comfortably covers it: the pool pays out
    // FASTER as it de-stresses.
    let env = new_env();
    let s = setup(&env);
    let anchor_staker = new_funded_address(&env, &s, MAX_STAKE);
    let anchor_ben = Address::generate(&env);
    s.client.stake(&anchor_staker, &MAX_STAKE, &anchor_ben);

    let cap_base = MAX_STAKE + MID_STAKE;
    let entitlement = bps_of(cap_base, JUST_ABOVE_FIRST_BAND_UTILISATION_BPS);
    assert!(entitlement <= stress_cap_oracle(cap_base, 0), "must clear admission");
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested

    let first = s.client.claim_stream(&claim_id, &ben);
    advance_days(&env, 1);
    let second = s.client.claim_stream(&claim_id, &ben);
    let expected_first = outflow_cap_oracle(cap_base, entitlement);
    let expected_second = outflow_cap_oracle(cap_base, entitlement - first);
    assert!(entitlement * BPS_DENOMINATOR / cap_base >= BAND_1_UTILISATION_BPS);
    assert!((entitlement - first) * BPS_DENOMINATOR / cap_base < BAND_1_UTILISATION_BPS);
    assert_eq!(first, expected_first);
    assert_eq!(second, expected_second);
    assert!(second > first, "the rate improved as utilisation fell");
}

// -----------------------------------------------------------------------
// cancel_claim
// -----------------------------------------------------------------------

#[test]
fn cancel_active_claim_restores_stake_with_penalty() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, ENTITLEMENT);
    s.client.cancel_claim(&claim_id);

    assert_eq!(s.client.get_total_staked(), MID_STAKE);
    assert_eq!(s.client.get_total_stakers(), 1);
    // Restored but penalty-locked: withdraw must still fail.
    let result = s.client.try_withdraw(&staker, &ben);
    assert_eq!(result, Err(Ok(PoolError::PenaltyLockActive)));
}

#[test]
fn cancel_active_claim_releases_total_allocated() {
    let env = new_env();
    let s = setup(&env);
    let (_staker, _ben, claim_id) = active_claim_with_entitlement(&env, &s, ENTITLEMENT);
    s.client.cancel_claim(&claim_id);
    assert_eq!(s.client.get_total_allocated(), 0);
}

#[test]
fn cancel_pending_claim_no_penalty() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.cancel_claim(&claim_id);
    // Stake was never forfeited (Pending), no penalty lock, withdraw
    // works immediately.
    s.client.withdraw(&staker, &ben);
}

#[test]
fn cancel_completed_claim_panics() {
    // Recalibrated 2026-09-19 -- same reasoning as
    // claim_stream_full_after_vesting_completes: needs an entitlement that
    // clears the outflow cap in one call so the claim actually reaches
    // Completed (a claim can only be "already completed" if it got there).
    let env = new_env();
    let s = setup(&env);
    let entitlement = SINGLE_CALL_ENTITLEMENT;
    let (_staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, entitlement);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS); // cooldown, then fully vested
    s.client.claim_stream(&claim_id, &ben);
    let result = s.client.try_cancel_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::ClaimNotCancellable)));
}

#[test]
fn cancel_nonexistent_claim_panics() {
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_cancel_claim(&tx_hash(&env, 99));
    assert_eq!(result, Err(Ok(PoolError::NoSuchClaim)));
}

#[test]
fn cancel_active_claim_allows_restake_after_penalty_lock() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben, claim_id) = active_claim_with_entitlement(&env, &s, ENTITLEMENT);
    s.client.cancel_claim(&claim_id);
    advance_ledgers(&env, PENALTY_LOCK_LEDGERS + 1); // penalty lock clears
    s.client.withdraw(&staker, &ben);
}
