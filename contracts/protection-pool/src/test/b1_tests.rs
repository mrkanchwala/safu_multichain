//! 2026-10-16: one test per new rule.
//! - the claim queue is a line, and each claim gets a fair share of
//!   the day's payout cap, so calling first or fast buys nothing.
//! - no expiry clock runs while the pool can't pay.
//! - pause gap, and a suspension by the admin alone lapses.
//! - upgrade needs two roles and 7 days; storage version + migrate.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};

use super::common::*;
use crate::error::PoolError;
use crate::types::{ClaimStatus, APPROVE_WINDOW_LEDGERS, COOLDOWN_LEDGERS, VESTING_LEDGERS};
use crate::{GovChange, GovKind};

const TIER_C: u32 = 3;

/// A claim filed now for `wallet`.
fn file(env: &Env, s: &Setup<'_>, wallet: &Address, seed: u8, entitlement: i128) -> BytesN<32> {
    submit_claim_signed(env, s, &s.oracle, wallet, &tx_hash(env, seed), &entitlement, &TIER_C, &now_ts(env))
}

// -----------------------------------------------------------------------
// line order
// -----------------------------------------------------------------------

#[test]
fn queued_claims_release_in_line_order_only() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _) = staked_wallet(&env, &s);
    let (w2, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let over_cap = expected_stress_cap(&s) + 1;
    let first = file(&env, &s, &w1, 1, over_cap);
    let second = file(&env, &s, &w2, 2, over_cap);
    assert_eq!(s.client.get_claim(&first).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_claim(&second).unwrap().status, ClaimStatus::Reserved);

    // Grow the pool until both fit. The second in line still can't go first.
    while expected_stress_cap(&s) < over_cap * 2 {
        staked_wallet(&env, &s);
    }
    assert_eq!(
        s.client.try_try_release_queued_claim(&second),
        Err(Ok(PoolError::NotFirstInQueue))
    );
    s.client.try_release_queued_claim(&first);
    assert_ne!(s.client.get_claim(&first).unwrap().status, ClaimStatus::Reserved);
    // Now it is next.
    s.client.try_release_queued_claim(&second);
    assert_ne!(s.client.get_claim(&second).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_queue(), (3, 3));
}

#[test]
fn a_new_claim_joins_the_back_even_when_there_is_room() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _) = staked_wallet(&env, &s);
    let (w2, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let big = file(&env, &s, &w1, 1, expected_stress_cap(&s) + 1);
    assert_eq!(s.client.get_claim(&big).unwrap().status, ClaimStatus::Reserved);

    // Small enough to be admitted on its own, but someone is in line.
    let small = file(&env, &s, &w2, 2, STROOPS_PER_UNIT);
    let claim = s.client.get_claim(&small).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    assert_eq!(claim.queue_seq, s.client.get_claim(&big).unwrap().queue_seq + 1);
    assert_eq!(
        s.client.try_try_release_queued_claim(&small),
        Err(Ok(PoolError::NotFirstInQueue))
    );
}

#[test]
fn a_dead_claim_at_the_front_does_not_stall_the_line() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _) = staked_wallet(&env, &s);
    let (w2, _) = staked_wallet(&env, &s);
    let (_w3, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let over_cap = expected_stress_cap(&s) + 1;
    let first = file(&env, &s, &w1, 1, over_cap);
    let second = file(&env, &s, &w2, 2, STROOPS_PER_UNIT);

    // An override pays w1 under another tx and forfeits its stake: the queued
    // claim can never go, so it's cleared and the next in line moves up.
    s.client.approve_override(&s.admin, &w1, &tx_hash(&env, 9), &MIN_STAKE, &TIER_C);
    s.client.approve_override(&s.co_signer, &w1, &tx_hash(&env, 9), &MIN_STAKE, &TIER_C);
    s.client.expire_queued_claim(&first);
    assert_eq!(s.client.get_claim(&first).unwrap().status, ClaimStatus::Expired);
    s.client.try_release_queued_claim(&second);
    assert_ne!(s.client.get_claim(&second).unwrap().status, ClaimStatus::Reserved);
}

// -----------------------------------------------------------------------
// fair daily shares
// -----------------------------------------------------------------------

/// Two equal claims, fully vested, collected on the same day in the given
/// order. Returns (paid to A, paid to B).
fn same_day_collections(a_first: bool) -> (i128, i128) {
    let env = new_env();
    let s = setup(&env);
    let (wa, ba) = staked_wallet(&env, &s);
    let (wb, bb) = staked_wallet(&env, &s);
    for _ in 0..4 {
        staked_wallet(&env, &s);
    }
    advance_past_time_gate(&env);
    let entitlement = MID_STAKE;
    let a = file(&env, &s, &wa, 1, entitlement);
    advance_days(&env, 1);
    let b = file(&env, &s, &wb, 2, entitlement);
    s.client.approve_claim(&a);
    s.client.approve_claim(&b);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    if a_first {
        let pa = s.client.claim_stream(&a, &ba);
        let pb = s.client.claim_stream(&b, &bb);
        (pa, pb)
    } else {
        let pb = s.client.claim_stream(&b, &bb);
        let pa = s.client.claim_stream(&a, &ba);
        (pa, pb)
    }
}

#[test]
fn calling_first_in_the_day_takes_nothing_from_the_others() {
    let (a_when_first, b_when_second) = same_day_collections(true);
    let (a_when_second, b_when_first) = same_day_collections(false);
    assert_eq!(a_when_first, a_when_second);
    assert_eq!(b_when_first, b_when_second);
    // The cap binds (otherwise this test proves nothing): neither got it all.
    assert!(a_when_first < MID_STAKE);
    assert!(b_when_first > 0);
}

#[test]
fn a_used_up_share_waits_for_tomorrow_and_nothing_is_lost() {
    let env = new_env();
    let s = setup(&env);
    let (wa, ba) = staked_wallet(&env, &s);
    for _ in 0..5 {
        staked_wallet(&env, &s);
    }
    advance_past_time_gate(&env);
    let entitlement = MID_STAKE;
    let a = file(&env, &s, &wa, 1, entitlement);
    s.client.approve_claim(&a);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    let day1 = s.client.claim_stream(&a, &ba);
    assert!(day1 < entitlement);
    assert_eq!(
        s.client.try_claim_stream(&a, &ba),
        Err(Ok(PoolError::DailyOutflowCapReached))
    );
    advance_days(&env, 1);
    let day2 = s.client.claim_stream(&a, &ba);
    assert!(day2 > 0);
    assert_eq!(s.client.get_claim(&a).unwrap().streamed, day1 + day2);
}

// -----------------------------------------------------------------------
// no expiry clock runs while the pool can't pay
// -----------------------------------------------------------------------

fn awaiting_approval(env: &Env, s: &Setup<'_>) -> BytesN<32> {
    let (w, _) = staked_wallet(env, s);
    advance_past_time_gate(env);
    let id = file(env, s, &w, 1, STROOPS_PER_UNIT);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::AwaitingApproval);
    id
}

#[test]
fn paused_days_do_not_count_toward_the_approve_window() {
    let env = new_env();
    let s = setup(&env);
    let id = awaiting_approval(&env, &s);
    advance_days(&env, 50);
    s.client.pause();
    advance_days(&env, 30); // pause runs out on its own
    assert!(!s.client.is_paused());
    advance_days(&env, 21); // 101 days since opening, 71 of them payable
    assert_eq!(
        s.client.try_expire_pending_approval(&id),
        Err(Ok(PoolError::ApprovalWindowNotExpired))
    );
    s.client.approve_claim(&id); // still on time
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
}

#[test]
fn approve_window_still_expires_on_payable_days() {
    let env = new_env();
    let s = setup(&env);
    let id = awaiting_approval(&env, &s);
    s.client.pause();
    advance_days(&env, 30);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + 1); // a full window of payable days
    s.client.expire_pending_approval(&id);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Expired);
}

#[test]
fn nothing_expires_while_the_pool_is_paused() {
    let env = new_env();
    let s = setup(&env);
    let id = awaiting_approval(&env, &s);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + 1);
    s.client.pause();
    assert_eq!(s.client.try_expire_pending_approval(&id), Err(Ok(PoolError::Paused)));
}

// -----------------------------------------------------------------------
// pause gap, suspension lapse + second role
// -----------------------------------------------------------------------

#[test]
fn unpausing_early_still_starts_the_gap() {
    let env = new_env();
    let s = setup(&env);
    s.client.pause();
    advance_days(&env, 1);
    s.client.unpause();
    assert_eq!(s.client.try_pause(), Err(Ok(PoolError::PauseGapNotMet)));
    advance_days(&env, 30);
    s.client.pause();
    // Paused time so far: the 1-day pause plus nothing of the new one yet.
    assert_eq!(s.client.get_blocked_secs(), crate::types::SECONDS_PER_DAY);
}

#[test]
fn the_admin_alone_cannot_chain_suspensions() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    s.client.suspend_stake(&w);
    assert_eq!(s.client.try_suspend_stake(&w), Err(Ok(PoolError::SuspendNeedsSecondRole)));
    advance_days(&env, 31); // lapsed, but within the gap
    assert!(s.client.is_eligible(&w));
    assert_eq!(s.client.try_suspend_stake(&w), Err(Ok(PoolError::SuspendNeedsSecondRole)));
    // A second role can renew.
    assert_eq!(
        s.client.try_renew_suspension(&w, &s.oracle),
        Err(Ok(PoolError::ApproverNotSecondRole))
    );
    s.client.renew_suspension(&w, &s.co_signer);
    assert!(!s.client.is_eligible(&w));
    advance_days(&env, 30);
    assert!(s.client.is_eligible(&w)); // lapses again on its own
}

// -----------------------------------------------------------------------
// upgrade path + storage version
// -----------------------------------------------------------------------

#[test]
fn an_upgrade_needs_two_roles_and_the_full_wait_even_before_funding() {
    let env = new_env();
    let s = setup(&env);
    let hash = BytesN::from_array(&env, &[7u8; 32]);
    let change = GovChange::Upgrade(hash);
    // Never the one-role recovery path.
    assert_eq!(
        s.client.try_propose_recovery(&s.admin, &change),
        Err(Ok(PoolError::GovRecoveryRoleOnly))
    );
    s.client.propose_change(&s.admin, &change);
    assert_eq!(s.client.try_execute_change(&GovKind::Upgrade), Err(Ok(PoolError::GovNotReady)));
    s.client.approve_change(&s.co_signer, &change);
    // The pool has never held money, yet the 7-day wait still applies.
    let eta = s.client.get_pending_change(&GovKind::Upgrade).unwrap().eta;
    assert_eq!(eta, now_ts(&env) + crate::governance::GOV_DELAY_SECONDS);
    assert_eq!(s.client.try_execute_change(&GovKind::Upgrade), Err(Ok(PoolError::GovNotReady)));
}

#[test]
fn storage_version_is_set_at_deploy_and_migrate_is_a_no_op_twice() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(s.client.get_storage_version(), crate::types::STORAGE_VERSION);
    assert_eq!(s.client.migrate(), crate::types::STORAGE_VERSION);
    assert_eq!(s.client.migrate(), crate::types::STORAGE_VERSION);
    let _ = Address::generate(&env);
}

// -----------------------------------------------------------------------
// Settings added 2026-10-16: bounds, order, and what existing stakes keep
// -----------------------------------------------------------------------

use crate::settings::SettingKey as K;

const DAY_SECS: i128 = crate::types::SECONDS_PER_DAY as i128;
const DAY_LEDGERS: i128 = crate::types::LEDGERS_PER_DAY as i128;
const TIER_A: u32 = 1;

fn try_file(
    env: &Env,
    s: &Setup<'_>,
    wallet: &Address,
    seed: u8,
    entitlement: i128,
    tier: u32,
    hack_ts: u64,
) -> Result<BytesN<32>, PoolError> {
    let deadline = default_deadline(env);
    let sig =
        sign_approval(env, s, wallet, &tx_hash(env, seed), &entitlement, &tier, &hack_ts, &deadline);
    match s.client.try_submit_claim(
        &s.oracle, wallet, &tx_hash(env, seed), &entitlement, &tier, &hack_ts, &deadline, &sig,
    ) {
        Ok(Ok(id)) => Ok(id),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected {:?}", other),
    }
}

#[test]
fn new_settings_refuse_values_outside_their_bounds() {
    let env = new_env();
    let s = setup(&env);
    for (key, bad) in [
        (K::TierARatio, 21),
        (K::TierCRatio, 0),
        (K::TimeGateLedgers, 90 * DAY_LEDGERS - 1),
        (K::ClaimWindowSecs, 14 * DAY_SECS - 1),
        (K::ClaimWindowSecs, 90 * DAY_SECS + 1),
        (K::ApproveWindowSecs, 30 * DAY_SECS - 1),
        (K::InactivitySecs, 365 * DAY_SECS + 1),
        (K::BackerMaturitySecs, 7 * DAY_SECS + 1),
    ] {
        assert_eq!(
            s.client.try_propose_setting(&key, &bad),
            Err(Ok(PoolError::SettingOutOfBounds)),
            "{:?} = {}",
            key,
            bad
        );
    }
}

#[test]
fn tier_ceilings_stay_in_order() {
    let env = new_env();
    let s = setup(&env);
    // C above B (10) is refused at execution.
    s.client.propose_setting(&K::TierCRatio, &11);
    s.client.approve_setting(&K::TierCRatio, &11);
    advance_ledgers(&env, (crate::settings::SETTINGS_TIMELOCK_SECONDS / SECONDS_PER_LEDGER) as u32 + 1);
    assert_eq!(s.client.try_execute_setting(&K::TierCRatio), Err(Ok(PoolError::SettingOrderInvalid)));
}

#[test]
fn a_ceiling_cut_reaches_new_stakes_only() {
    let env = new_env();
    let s = setup(&env);
    let (old, _) = staked_wallet(&env, &s);
    set_setting_via_timelock(&env, &s, K::TierARatio, 12);
    let (new, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let full = MID_STAKE * 15;
    assert!(try_file(&env, &s, &old, 1, full, TIER_A, now_ts(&env)).is_ok());
    assert_eq!(
        try_file(&env, &s, &new, 2, full, TIER_A, now_ts(&env)),
        Err(PoolError::EntitlementExceedsTierCap)
    );
    assert!(try_file(&env, &s, &new, 3, MID_STAKE * 12, TIER_A, now_ts(&env)).is_ok());
}

#[test]
fn a_ceiling_raise_reaches_existing_stakes() {
    let env = new_env();
    let s = setup(&env);
    let (old, _) = staked_wallet(&env, &s);
    set_setting_via_timelock(&env, &s, K::TierCRatio, 8);
    advance_past_time_gate(&env);
    assert!(try_file(&env, &s, &old, 1, MID_STAKE * 8, TIER_C, now_ts(&env)).is_ok());
}

#[test]
fn a_longer_time_gate_does_not_reach_existing_stakes() {
    let env = new_env();
    let s = setup(&env);
    let (old, _) = staked_wallet(&env, &s);
    set_setting_via_timelock(&env, &s, K::TimeGateLedgers, 120 * DAY_LEDGERS);
    let (new, _) = staked_wallet(&env, &s);
    advance_days(&env, 91);
    assert!(s.client.is_claim_eligible(&old));
    assert!(!s.client.is_claim_eligible(&new));
    advance_days(&env, 30);
    assert!(s.client.is_claim_eligible(&new));
}

#[test]
fn a_shorter_claim_window_does_not_reach_existing_stakes() {
    let env = new_env();
    let s = setup(&env);
    let (old, _) = staked_wallet(&env, &s);
    set_setting_via_timelock(&env, &s, K::ClaimWindowSecs, 14 * DAY_SECS);
    let (new, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let hack = now_ts(&env);
    advance_days(&env, 20); // past 14, inside 30
    assert!(try_file(&env, &s, &old, 1, STROOPS_PER_UNIT, TIER_C, hack).is_ok());
    assert_eq!(
        try_file(&env, &s, &new, 2, STROOPS_PER_UNIT, TIER_C, hack),
        Err(PoolError::ClaimWindowExpired)
    );
}

#[test]
fn a_shorter_approve_window_does_not_reach_a_claim_already_waiting() {
    let env = new_env();
    let s = setup(&env);
    let id = awaiting_approval(&env, &s); // entered with the 100-day window
    set_setting_via_timelock(&env, &s, K::ApproveWindowSecs, 30 * DAY_SECS);
    advance_days(&env, 50);
    assert_eq!(
        s.client.try_expire_pending_approval(&id),
        Err(Ok(PoolError::ApprovalWindowNotExpired))
    );
    assert_eq!(s.client.get_claim(&id).unwrap().window_secs, crate::types::APPROVE_WINDOW_SECONDS);
}

#[test]
fn backer_maturity_is_a_setting() {
    let env = new_env();
    let s = setup(&env);
    set_setting_via_timelock(&env, &s, K::BackerMaturitySecs, 0);
    let backer = Address::generate(&env);
    s.token_admin.mint(&backer, &MID_STAKE);
    s.client.back(&backer, &MID_STAKE);
    // Matures at once, no wait.
    s.client.mature_backing(&backer);
    assert_eq!(s.client.get_total_backed(), MID_STAKE);
}
