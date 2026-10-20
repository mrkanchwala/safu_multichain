//! 2026-10-16: one test per new rule.
//! - the claim queue is a line, and each claim gets a fair share of
//!   the day's payout cap, so calling first or fast buys nothing.
//! - no expiry clock runs while the pool can't pay.
//! - pause gap, and a suspension by the admin alone lapses.
//! - upgrade needs two roles and 7 days; storage version + migrate.
//! - No keeper. Approve is one click after day 90,
//!   the approve window starts at day 90 by itself, and every user call
//!   releases the line and clears dead claims.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};

use super::common::*;
use crate::error::PoolError;
use crate::types::{
    ClaimStatus, APPROVE_WINDOW_LEDGERS, COLLECTION_INACTIVITY_LEDGERS, COOLDOWN_LEDGERS, LEDGERS_PER_DAY,
    VESTING_LEDGERS,
};
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

    // Grow the pool until both fit. Each new stake releases the first in line
    // by itself; the second never goes while the first waits.
    let mut stakes = 0;
    while s.client.get_claim(&second).unwrap().status == ClaimStatus::Reserved {
        staked_wallet(&env, &s);
        stakes += 1;
        assert!(stakes < 200, "line never cleared");
        if s.client.get_claim(&first).unwrap().status == ClaimStatus::Reserved {
            assert_eq!(s.client.get_claim(&second).unwrap().status, ClaimStatus::Reserved);
        }
    }
    assert_ne!(s.client.get_claim(&first).unwrap().status, ClaimStatus::Reserved);
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
    // The gap is strict: exactly 30 days after the end is still too soon.
    assert_eq!(s.client.try_pause(), Err(Ok(PoolError::PauseGapNotMet)));
    advance_ledgers(&env, 1);
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
    // Default 0: counts at once.
    let backer = Address::generate(&env);
    s.token_admin.mint(&backer, &(2 * MID_STAKE));
    s.client.back(&backer, &MID_STAKE);
    assert_eq!(s.client.get_total_backed(), MID_STAKE);
    // Set to a day: the next deposit waits.
    set_setting_via_timelock(&env, &s, K::BackerMaturitySecs, DAY_SECS);
    s.client.back(&backer, &MID_STAKE);
    assert_eq!(s.client.get_total_backed_pending(), MID_STAKE);
    assert_eq!(s.client.try_mature_backing(&backer), Err(Ok(PoolError::BackingNotMature)));
}

// -- Stake overflow, re-stake, drip settings ----

#[test]
fn a_deposit_above_the_max_stakes_the_max_and_backs_the_rest() {
    let env = new_env();
    let s = setup(&env);
    let extra = 3 * MID_STAKE;
    let staker = new_funded_address(&env, &s, MAX_STAKE + extra);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &(MAX_STAKE + extra), &ben);
    assert_eq!(s.client.get_stake(&staker).unwrap().amount, MAX_STAKE);
    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    let b = s.client.get_backer(&staker).unwrap();
    assert_eq!(b.amount, extra, "the rest is backing, counted at once");
    assert_eq!(s.client.get_capacity(), MAX_STAKE + extra);
    let token = soroban_sdk::token::TokenClient::new(&env, &s.token_id);
    assert_eq!(token.balance(&staker), 0, "one transfer for both parts");
}

#[test]
fn a_full_pool_refuses_the_whole_deposit() {
    let env = new_env();
    let s = setup_with_cap(&env, POOL_CAP);
    // Fill the pool's staking room to within less than one max stake.
    let mut room = POOL_CAP - s.client.get_total_staked();
    while room >= MAX_STAKE {
        staked_wallet_amount(&env, &s, MAX_STAKE);
        room = POOL_CAP - s.client.get_total_staked();
    }
    let staker = new_funded_address(&env, &s, 2 * MAX_STAKE);
    let ben = Address::generate(&env);
    assert_eq!(s.client.try_stake(&staker, &(2 * MAX_STAKE), &ben), Err(Ok(PoolError::PoolCapExceeded)));
    assert!(s.client.get_backer(&staker).is_none(), "nothing turned into backing");
}

#[test]
fn below_the_min_is_still_refused() {
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MIN_STAKE);
    let ben = Address::generate(&env);
    assert_eq!(s.client.try_stake(&staker, &(MIN_STAKE - 1), &ben), Err(Ok(PoolError::StakeOutOfRange)));
}

#[test]
fn no_top_ups_even_above_the_max() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.token_admin.mint(&staker, &(2 * MAX_STAKE));
    assert_eq!(s.client.try_stake(&staker, &(2 * MAX_STAKE), &ben), Err(Ok(PoolError::AlreadyStaked)));
    assert!(s.client.get_backer(&staker).is_none());
}

#[test]
fn restake_rule_new_address_after_a_paid_claim_same_address_otherwise() {
    // After an approved claim the address never
    // stakes again (the registry keeps its wallets bound to it, so a fresh
    // address with new wallets is the only way back in). Without a paid
    // claim, withdraw + stake again works.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.withdraw(&staker, &ben);
    s.token_admin.mint(&staker, &MID_STAKE);
    s.client.stake(&staker, &MID_STAKE, &ben);

    advance_past_time_gate(&env);
    let id = submit_claim_signed(&env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &(MID_STAKE / 10), &1, &now_ts(&env));
    s.client.approve_claim(&id);
    s.token_admin.mint(&staker, &MID_STAKE);
    assert_eq!(s.client.try_stake(&staker, &MID_STAKE, &ben), Err(Ok(PoolError::AddressHasApprovedClaim)));
}

#[test]
fn drip_settings_have_bounds_and_order() {
    let env = new_env();
    let s = setup(&env);
    for (key, bad) in [
        (K::DripMinSecs, 0),
        (K::DripMinSecs, 90 * DAY_SECS + 1),
        (K::DripMaxSecs, 7 * DAY_SECS - 1),
        (K::DripMaxSecs, 180 * DAY_SECS + 1),
    ] {
        assert_eq!(s.client.try_propose_setting(&key, &bad), Err(Ok(PoolError::SettingOutOfBounds)));
    }
    // Min above the live max (90 d) is refused at execution.
    s.client.propose_setting(&K::DripMinSecs, &(90 * DAY_SECS));
    s.client.approve_setting(&K::DripMinSecs, &(90 * DAY_SECS));
    set_setting_via_timelock(&env, &s, K::DripMaxSecs, 30 * DAY_SECS);
    assert_eq!(s.client.try_execute_setting(&K::DripMinSecs), Err(Ok(PoolError::SettingOrderInvalid)));
}

// -----------------------------------------------------------------------
// No keeper
// -----------------------------------------------------------------------

/// A claim filed before day 90 for `w`, small enough to be admitted.
fn early_claim(env: &Env, s: &Setup<'_>, w: &Address, seed: u8) -> BytesN<32> {
    let id = file(env, s, w, seed, STROOPS_PER_UNIT);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::PendingTime);
    id
}

#[test]
fn approve_is_one_click_after_day_90() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    let id = early_claim(&env, &s, &w, 1);
    advance_past_time_gate(&env);
    // Nobody called unlock: the staker's approve does it.
    s.client.approve_claim(&id);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
}

#[test]
fn the_approve_window_starts_at_day_90_by_itself() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    let id = early_claim(&env, &s, &w, 1);
    // 99 days after day 90: still inside the window counted from day 90.
    advance_past_time_gate(&env);
    advance_days(&env, 99);
    s.client.approve_claim(&id);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);

    let (w2, _) = staked_wallet(&env, &s);
    let id2 = early_claim(&env, &s, &w2, 2);
    // 101 days after day 90, nobody ever unlocked it: the window ran out.
    advance_past_time_gate(&env);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + LEDGERS_PER_DAY);
    assert_eq!(s.client.try_approve_claim(&id2), Err(Ok(PoolError::ApprovalWindowExpired)));
}

#[test]
fn an_unapproved_claim_is_cleared_by_anyone_elses_call() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    let id = early_claim(&env, &s, &w, 1);
    assert_eq!(s.client.get_total_allocated(), STROOPS_PER_UNIT);
    advance_past_time_gate(&env);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + LEDGERS_PER_DAY);
    // An unrelated staker joins; their call clears the dead claim.
    staked_wallet(&env, &s);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Expired);
    assert_eq!(s.client.get_total_allocated(), 0);
    assert!(s.client.get_stake(&w).unwrap().active_claim_id.is_none());
}

#[test]
fn an_uncollected_claim_is_cleared_by_anyone_elses_call() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let id = file(&env, &s, &w, 1, STROOPS_PER_UNIT);
    s.client.approve_claim(&id);
    assert_eq!(s.client.get_total_allocated(), STROOPS_PER_UNIT);
    // Never collected: the inactivity clock starts at cooldown end.
    advance_ledgers(&env, COOLDOWN_LEDGERS + COLLECTION_INACTIVITY_LEDGERS + LEDGERS_PER_DAY);
    staked_wallet(&env, &s);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Expired);
    assert_eq!(s.client.get_total_allocated(), 0);
}

#[test]
fn a_live_claim_is_never_cleared_early() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    let id = early_claim(&env, &s, &w, 1);
    advance_past_time_gate(&env);
    advance_days(&env, 50);
    // Many unrelated calls: the claim is checked again and again, and kept.
    for _ in 0..6 {
        staked_wallet(&env, &s);
    }
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::AwaitingApproval);
    assert_eq!(s.client.get_total_allocated(), STROOPS_PER_UNIT);
    s.client.approve_claim(&id);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
}

// -----------------------------------------------------------------------
// Points are dollar-days
// -----------------------------------------------------------------------

#[test]
fn a_points_rate_change_never_rescales_past_points() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_days(&env, 10);
    let before = s.client.get_points_balance(&w);
    assert_eq!(before, dollar_day_points(MID_STAKE, 10));
    // Double the staker rate; the 7-day timelock runs at the old rate.
    set_setting_via_timelock(&env, &s, crate::settings::SettingKey::PointsRateStaker, 20_000);
    let at_change = s.client.get_points_balance(&w);
    assert!(at_change >= before + dollar_day_points(MID_STAKE, 7));
    assert!(at_change <= before + dollar_day_points(MID_STAKE, 8));
    advance_days(&env, 10);
    assert_eq!(s.client.get_points_balance(&w), at_change + 2 * dollar_day_points(MID_STAKE, 10));
}

#[test]
fn backers_earn_points_on_their_own_rate() {
    let env = new_env();
    let s = setup(&env);
    let backer = new_funded_address(&env, &s, MID_STAKE);
    s.client.back(&backer, &MID_STAKE);
    advance_days(&env, 20);
    assert_eq!(s.client.get_backer_points(&backer), dollar_day_points(MID_STAKE, 20));
    assert_eq!(s.client.get_points_balance(&backer), 0, "staker points are separate");
}

// -----------------------------------------------------------------------
// What the upkeep costs the user (run with --nocapture to print)
// -----------------------------------------------------------------------

/// Fee estimate of the last call, in stroops: (total, rent, everything
/// except rent). Pubnet fee snapshot from the SDK; native test contract, so
/// VM costs are missing, but they are the same with or without upkeep.
fn last_fee(env: &Env) -> (i64, i64, i64) {
    let f = env.cost_estimate().fee();
    let rent = f.persistent_entry_rent + f.temporary_entry_rent;
    (f.total, rent, f.total - rent)
}

/// Stake fee after `days`, with `claims` early claims filed first and
/// `dead` deciding whether they have run out by then.
fn stake_fee(claims: u8, dead: bool) -> (i64, i64, i64) {
    let env = new_env();
    let s = setup(&env);
    let mut ids = std::vec::Vec::new();
    for seed in 1..=claims {
        let (w, _) = staked_wallet(&env, &s);
        ids.push(early_claim(&env, &s, &w, seed));
    }
    if claims == 0 {
        staked_wallet(&env, &s);
        staked_wallet(&env, &s);
    }
    // Same time passes in every case, so storage rent is comparable.
    advance_past_time_gate(&env);
    if dead {
        advance_ledgers(&env, APPROVE_WINDOW_LEDGERS + LEDGERS_PER_DAY);
    } else {
        advance_days(&env, 50);
    }
    staked_wallet(&env, &s);
    let fee = last_fee(&env);
    for id in ids {
        let status = s.client.get_claim(&id).unwrap().status;
        assert_eq!(status == ClaimStatus::Expired, dead);
    }
    fee
}

#[test]
fn upkeep_fee_stays_small() {
    let (idle_dead_time, idle_live_time) = (stake_fee(0, true), stake_fee(0, false));
    let checked = stake_fee(2, false);
    let cleared = stake_fee(2, true);
    std::println!("stake fee in stroops (total, rent, non-rent); 1 XLM = 10,000,000:");
    std::println!("  nothing to tidy, day 140: {idle_live_time:?}");
    std::println!("  2 claims checked, day 140: {checked:?}  non-rent +{}", checked.2 - idle_live_time.2);
    std::println!("  nothing to tidy, day 191: {idle_dead_time:?}");
    std::println!("  2 claims cleared, day 191: {cleared:?}  non-rent +{}", cleared.2 - idle_dead_time.2);
    // The upkeep's own work (not rent) adds under 0.01 XLM per call.
    assert!(checked.2 - idle_live_time.2 < 100_000);
    assert!(cleared.2 - idle_dead_time.2 < 100_000);
}
