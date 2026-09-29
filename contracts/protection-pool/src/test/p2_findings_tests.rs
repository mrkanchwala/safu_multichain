//! Pre-audit gate P2 (2026-09-23): regression tests for the two findings of
//! the v1 `/audit-chain` pass. Each test asserts the CORRECT behaviour.
//!
//! H1: a queued (`Reserved`) claim did not block `withdraw`/`emergency_exit`,
//! and neither release nor approval re-checked the stake, so a staker could
//! take their principal back and still be paid in full.
//!
//! H2: staking again after an approved claim overwrote the stake record
//! (clearing `suspended`, and letting a later `cancel_claim` restore money
//! that had already left). Founder rule 2026-09-23: once a claim is approved,
//! that address can never stake again.

#![cfg(test)]

use super::common::*;
use crate::error::PoolError;
use crate::types::ClaimStatus;
use soroban_sdk::testutils::Address as _;

const TIER_C: u32 = 3;

/// Small enough to be admitted at once (under the stress cap), within Tier C.
fn admitted_entitlement() -> i128 {
    MID_STAKE / 10
}

/// Stakes, passes the time gate, and queues a claim over the stress cap.
fn queued_claim(
    env: &soroban_sdk::Env,
    s: &Setup<'_>,
) -> (soroban_sdk::Address, soroban_sdk::Address, soroban_sdk::BytesN<32>, i128) {
    let (w, b) = staked_wallet(env, s);
    advance_past_time_gate(env);
    let entitlement = expected_stress_cap(s) + 1;
    let claim_id = submit_claim_signed(
        env, s, &s.oracle, &w, &tx_hash(env, 1), &entitlement, &TIER_C, &now_ts(env),
    );
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);
    (w, b, claim_id, entitlement)
}

// -- H1 --

#[test]
fn withdraw_is_refused_while_a_claim_is_queued() {
    let env = new_env();
    let s = setup(&env);
    let (w, b, _id, _e) = queued_claim(&env, &s);
    assert_eq!(s.client.try_withdraw(&w, &b), Err(Ok(PoolError::ClaimQueuedForStake)));
}

#[test]
fn emergency_exit_is_refused_while_a_claim_is_queued() {
    let env = new_env();
    let s = setup(&env);
    let (w, _b, _id, _e) = queued_claim(&env, &s);
    assert_eq!(s.client.try_emergency_exit(&w), Err(Ok(PoolError::ClaimQueuedForStake)));
}

/// End to end: principal out, then the queued claim released and approved.
/// It must never reach `Active` for a stake whose principal already left.
#[test]
fn principal_withdrawn_while_queued_never_becomes_a_paying_claim() {
    let env = new_env();
    let s = setup(&env);
    let (w, b, claim_id, entitlement) = queued_claim(&env, &s);

    let withdrew = s.client.try_withdraw(&w, &b).is_ok();
    // Keep the pool non-empty so the stress-cap helper never divides by 0.
    staked_wallet(&env, &s);

    // Grow the pool until the queued entitlement fits today's stress cap.
    while expected_stress_cap(&s) < entitlement {
        staked_wallet(&env, &s);
    }
    let _ = s.client.try_try_release_queued_claim(&claim_id);
    let _ = s.client.try_approve_claim(&claim_id);

    let status = s.client.get_claim(&claim_id).unwrap().status;
    assert!(
        !(withdrew && status == ClaimStatus::Active),
        "principal withdrawn AND claim active: paid without forfeiting"
    );
}

// -- H2 --

#[test]
fn no_new_stake_after_an_approved_claim() {
    let env = new_env();
    let s = setup(&env);
    let (w, _b) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w, &tx_hash(&env, 1), &admitted_entitlement(), &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Active);

    s.token_admin.mint(&w, &MID_STAKE);
    let new_ben = soroban_sdk::Address::generate(&env);
    assert_eq!(
        s.client.try_stake(&w, &MID_STAKE, &new_ben),
        Err(Ok(PoolError::AddressHasApprovedClaim))
    );
}

/// A suspended claimant must not lift their own suspension by staking again.
#[test]
fn staking_again_cannot_lift_a_suspension() {
    let env = new_env();
    let s = setup(&env);
    let (w, _b) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w, &tx_hash(&env, 1), &admitted_entitlement(), &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);
    s.client.suspend_stake(&w);

    s.token_admin.mint(&w, &MID_STAKE);
    let _ = s.client.try_stake(&w, &MID_STAKE, &soroban_sdk::Address::generate(&env));
    assert!(s.client.get_stake(&w).unwrap().suspended);
}

// -- M2: admin actions are public --

#[test]
fn admin_actions_emit_events() {
    use soroban_sdk::testutils::Events as _;
    let env = new_env();
    let s = setup(&env);
    let (w, _b) = staked_wallet(&env, &s);

    s.client.pause();
    assert_eq!(env.events().all().events().len(), 1, "pause");
    s.client.unpause();
    assert_eq!(env.events().all().events().len(), 1, "unpause");
    s.client.suspend_stake(&w);
    assert_eq!(env.events().all().events().len(), 1, "suspend");
    s.client.unsuspend_stake(&w, &None);
    assert_eq!(env.events().all().events().len(), 1, "unsuspend");
    s.client.set_pool_cap(&s.client.get_total_staked());
    assert_eq!(env.events().all().events().len(), 1, "pool cap lowered");
    s.client.approve_override(&s.admin, &w, &tx_hash(&env, 9), &(MID_STAKE / 10), &TIER_C);
    assert_eq!(env.events().all().events().len(), 1, "first override approval");
}
