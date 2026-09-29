#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::error::PoolError;
use crate::types::{
    ClaimStatus, COOLDOWN_LEDGERS, PENALTY_LOCK_LEDGERS, TIER_C_RATIO, VESTING_LEDGERS,
};

const ENTITLEMENT: i128 = STROOPS_PER_UNIT;
const TIER_C: u32 = 3;

#[test]
fn stale_approval_from_rotated_cosigner_does_not_count() {
    // Regression test for a real bug found via adversarial review
    // (2026-07-14, Solodit "operator retains power after removal from
    // governance" pattern class): OverrideRequest used to store bare
    // owner_approved/co_signer_approved bools with no record of WHO
    // approved. If coSigner was rotated between the two approvals, the
    // OLD coSigner's stale `true` flag still counted toward readiness,
    // silently degrading the 2-of-2 property to "1 current + 1 stale"
    // after any coSigner rotation: exactly the scenario an admin would
    // hit revoking a suspected-compromised coSigner key.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);

    // Old coSigner approves first.
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);

    // Admin rotates coSigner out: e.g. because it's suspected compromised.
    let new_co_signer = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::CoSigner(new_co_signer.clone()));

    // Admin's approval alone must NOT execute, the stored approval was
    // from the OLD coSigner, which no longer matches the current one.
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    assert!(
        s.client.get_claim(&claim_id).is_none(),
        "stale coSigner approval must not combine with a fresh admin approval"
    );

    // The NEW coSigner approving (fresh, current) is what actually
    // completes the 2-of-2 and executes.
    s.client
        .approve_override(&new_co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
}

#[test]
fn stale_approval_from_transferred_admin_does_not_count() {
    // Same pattern, admin side: an admin's approval recorded BEFORE
    // transfer_admin must not combine with the coSigner's approval to
    // execute under the NEW admin's authority.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);

    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);

    let new_admin = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Admin(new_admin.clone()));

    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    assert!(s.client.get_claim(&claim_id).is_none());

    s.client
        .approve_override(&new_admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
}

#[test]
fn single_approval_does_not_execute() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    // execute_override creates the Claim record, no claim exists yet
    // with only one of the two approvals in.
    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    assert!(s.client.get_claim(&claim_id).is_none());
    // Stake untouched: still forfeitable in the normal flow, proving
    // the single approval had zero effect.
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
}

#[test]
fn second_matching_approval_executes() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);

    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
    assert_eq!(claim.entitlement, ENTITLEMENT);
    // Stake forfeited by the override, same as a normal activation.
    assert_eq!(s.client.get_total_staked(), 0);
}

#[test]
fn override_bypasses_time_gate() {
    // The whole point of the escape hatch: no 90-day wait, forfeits
    // immediately on the second approval regardless of stake age.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s); // staked THIS ledger, gate nowhere close to met
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);
    assert_eq!(s.client.get_total_staked(), 0);
}

#[test]
fn mismatched_second_approval_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let result =
        s.client
            .try_approve_override(&s.co_signer, &staker, &hash, &(ENTITLEMENT + 1), &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::OverrideParamsMismatch)));
}

#[test]
fn approve_override_wrong_caller_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let random = Address::generate(&env);
    let result = s
        .client
        .try_approve_override(&random, &staker, &tx_hash(&env, 1), &ENTITLEMENT, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::CallerNotAdminOrCoSigner)));
}

#[test]
fn approve_override_zero_entitlement_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = s
        .client
        .try_approve_override(&s.admin, &staker, &tx_hash(&env, 1), &0, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::EntitlementNotPositive)));
}

#[test]
fn approve_override_invalid_tier_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result = s
        .client
        .try_approve_override(&s.admin, &staker, &tx_hash(&env, 1), &ENTITLEMENT, &9);
    assert_eq!(result, Err(Ok(PoolError::InvalidTier)));
}

#[test]
fn approve_override_exceeds_tier_cap_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    let too_much = MID_STAKE * TIER_C_RATIO + 1; // one unit over tier C's cap for MID_STAKE
    s.client
        .approve_override(&s.admin, &staker, &hash, &too_much, &TIER_C);
    let result = s
        .client
        .try_approve_override(&s.co_signer, &staker, &hash, &too_much, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::EntitlementExceedsTierCap)));
}

#[test]
fn approve_override_insolvent_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    // Under tier cap (500M) but exceeds total_staked (100M), same
    // solvency invariant applies to the override path.
    let over_solvent = MID_STAKE + 1;
    s.client
        .approve_override(&s.admin, &staker, &hash, &over_solvent, &TIER_C);
    let result = s
        .client
        .try_approve_override(&s.co_signer, &staker, &hash, &over_solvent, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::Insolvent)));
}

#[test]
fn override_reexecution_same_params_resets_cooldown() {
    // KB §1b: re-executing on a still-Active (not Completed) claim is
    // allowed and gives it fresh cooldown/vesting deadlines, this is
    // the "correction" mechanism for identical-params re-approval.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);

    advance_days(&env, 3); // partway into the first cooldown

    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);

    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    // Total staked should NOT have decremented a second time, the stake
    // was already forfeited by the first execution.
    assert_eq!(s.client.get_total_staked(), 0);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
}

#[test]
fn override_on_completed_claim_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    let small = 900_000i128; // fully vests fast, low enough to clear outflow cap in one call

    s.client.approve_override(&s.admin, &staker, &hash, &small, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &small, &TIER_C);

    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    advance_ledgers(&env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    s.client.claim_stream(&claim_id, &ben);

    s.client.approve_override(&s.admin, &staker, &hash, &small, &TIER_C);
    let result = s
        .client
        .try_approve_override(&s.co_signer, &staker, &hash, &small, &TIER_C);
    assert_eq!(result, Err(Ok(PoolError::ClaimAlreadyCompleted)));
}

#[test]
fn cosigner_cannot_be_set_equal_to_admin() {
    // Corrected 2026-07-14: a full re-read of V8's setCoSigner found it
    // DOES check `newCoSigner != owner()` (this test previously assumed
    // the opposite and asserted the degenerate case was reachable, which
    // was never actually verified against source). Combined with
    // transferOwnership's `newOwner != coSigner` check and initialize's
    // constructor check, coSigner == admin is unreachable in V8 from
    // every direction: matched exactly here. The `|| co_signer == admin`
    // branch in claim.rs's override-readiness check is V8's own
    // defensive/dead code for a state that provably can't occur, kept
    // for exact parity rather than removed.
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_propose_change(&s.admin, &crate::GovChange::CoSigner(s.admin.clone()));
    assert_eq!(result, Err(Ok(PoolError::GovRoleNotDistinct)));
}

// -----------------------------------------------------------------------
// cancel_pending_override
// -----------------------------------------------------------------------

#[test]
fn cancel_pending_override_allows_corrected_resubmission() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client.cancel_pending_override(&s.admin, &staker, &hash);

    // Different entitlement now: would have panicked on "params
    // mismatch" before the cancel fix; must succeed post-cancel.
    let corrected = ENTITLEMENT + 500_000;
    s.client
        .approve_override(&s.admin, &staker, &hash, &corrected, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &corrected, &TIER_C);

    let claim_id = compute_test_claim_id(&env, &staker, &hash);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.entitlement, corrected);
}

#[test]
fn cancel_pending_override_wrong_caller_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let random = Address::generate(&env);
    let result = s.client.try_cancel_pending_override(&random, &staker, &hash);
    assert_eq!(result, Err(Ok(PoolError::CallerNotAdmin)));
}

#[test]
fn cancel_pending_override_cosigner_cannot_cancel() {
    // V8's cancelPendingOverride is onlyOwner: admin only. An earlier
    // draft here wrongly allowed coSigner too, before reading V8 directly.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let result = s
        .client
        .try_cancel_pending_override(&s.co_signer, &staker, &hash);
    assert_eq!(result, Err(Ok(PoolError::CallerNotAdmin)));
}

#[test]
fn cancel_pending_override_nonexistent_panics() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let result =
        s.client
            .try_cancel_pending_override(&s.admin, &staker, &tx_hash(&env, 99));
    assert_eq!(result, Err(Ok(PoolError::NoPendingOverride)));
}

#[test]
fn cancel_pending_override_after_execution_panics() {
    // Matches V8 exactly: execution DELETES the stored request (see
    // approve_override), so a post-execution cancel attempt naturally
    // fails with NoPendingOverride: the same error a never-existed
    // request would give, not a distinct "already executed" variant.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);
    let result = s.client.try_cancel_pending_override(&s.admin, &staker, &hash);
    assert_eq!(result, Err(Ok(PoolError::NoPendingOverride)));
}

/// Mutation-testing gap fix (2026-07-22 re-run). Kills claim.rs:827
/// (`||`->`&&`), collapses the 3-way status check to effectively
/// Active-only, which would silently skip releasing `total_allocated` for
/// an AwaitingApproval claim being overridden, double-counting the
/// reservation instead of replacing it.
#[test]
fn override_on_awaiting_approval_claim_releases_prior_reservation() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    advance_past_time_gate(&env); // gate met -> submit_claim lands AwaitingApproval directly
    let claim_id = submit_claim_signed(&env, &s, &s.oracle, &staker, &hash, &ENTITLEMENT, &TIER_C, &now_ts(&env));
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::AwaitingApproval);
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT);

    let smaller = 700_000i128;
    s.client
        .approve_override(&s.admin, &staker, &hash, &smaller, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &smaller, &TIER_C);

    // Old AwaitingApproval reservation released, only the override's own
    // entitlement remains allocated: not the sum of both.
    assert_eq!(s.client.get_total_allocated(), smaller);
}

#[test]
fn cancel_active_claim_then_override_does_not_double_release_allocation() {
    // Regression test for a real bug found reading V8 directly
    // (2026-07-14): execute_override used to release total_allocated for
    // ANY non-completed prior claim, including an already-Cancelled one,
    // but cancel_claim already released that same reservation itself.
    // Re-targeting a previously-cancelled wallet+tx_hash pair via
    // override would silently double-subtract total_allocated (masked
    // by a .max(0) clamp instead of caught). V8 only releases when
    // prevStatus is Active or Pending, never Cancelled, matched here.
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);

    // First cycle: normal claim, then admin cancels it (false positive).
    let claim_id = submit_claim_signed(&env, &s, &s.oracle, &staker, &hash, &ENTITLEMENT, &TIER_C, &now_ts(&env));
    s.client.cancel_claim(&claim_id);
    assert_eq!(s.client.get_total_allocated(), 0); // released once by cancel_claim

    // Second cycle: override re-targets the SAME wallet+tx_hash (now
    // Cancelled). Must forfeit fresh and allocate ENTITLEMENT exactly
    // once: not attempt to release a reservation that's already zero.
    s.client
        .approve_override(&s.admin, &staker, &hash, &ENTITLEMENT, &TIER_C);
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &ENTITLEMENT, &TIER_C);
    assert_eq!(s.client.get_total_allocated(), ENTITLEMENT);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Active);
}

#[test]
fn cancel_claim_restores_stake_without_needing_amount_field() {
    // Regression test: activate_claim no longer zeroes StakeRecord.amount
    // on claim-triggered forfeiture (matches V8, only withdrawn=true is
    // set; only voluntary withdraw() zeroes amount). cancel_claim
    // restoring `withdrawn=false` alone (no amount write) must be
    // sufficient for a full, correct withdrawal afterward.
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
    advance_ledgers(&env, PENALTY_LOCK_LEDGERS + 1); // penalty lock clears
    s.client.withdraw(&staker, &ben);
    assert_eq!(s.client.get_total_staked(), 0);
}

/// Test-only re-derivation of the on-chain claim id (sha256 of wallet
/// XDR ++ tx_hash bytes), mirrors claim.rs::compute_claim_id exactly so
/// tests can look up claims created via the override path without a
/// dedicated "last claim id" getter.
fn compute_test_claim_id(
    env: &soroban_sdk::Env,
    wallet: &Address,
    hash: &soroban_sdk::BytesN<32>,
) -> soroban_sdk::BytesN<32> {
    use soroban_sdk::xdr::ToXdr;
    let mut buf = wallet.to_xdr(env);
    buf.append(&soroban_sdk::Bytes::from_array(env, &hash.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}
