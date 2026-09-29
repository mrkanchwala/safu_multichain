//! D1 (Tranche 2), on-chain Ed25519 oracle approval verification.
//!
//! Covers the signature gate added to `submit_claim`, the `OraclePubKey`
//! attestation identity, and the `revoke_approval` flow. The pre-existing
//! 184 tests already traverse this gate on every oracle-path call (they were
//! migrated to `submit_claim_signed`, which produces a real signature); this
//! file tests the gate itself, what it accepts, what it rejects, and how it
//! fails.
//!
//! **Reading the two failure shapes.** They are not interchangeable and the
//! distinction is the whole point of the check ordering in
//! `claim::verify_oracle_signature`:
//!
//! - Every RECOVERABLE condition (expired, deadline too far, pubkey missing,
//!   revoked) returns a typed `PoolError` and is asserted as
//!   `Err(Ok(PoolError::X))`.
//! - A genuine CRYPTOGRAPHIC mismatch traps opaquely, `ed25519_verify`
//!   returns `()` and panics, with no `try_` variant at SDK 27 and no way to
//!   catch a host trap in-guest. Those are asserted with
//!   `assert_signature_trap`, which requires the failure to be a trap and
//!   NOT a typed error. That assertion is deliberately two-sided: if a future
//!   change made a crypto mismatch return a typed error, these tests should
//!   fail and be re-read, not silently pass.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, BytesN, ConversionError, Env, InvokeError, Symbol};

use super::common::*;
use crate::error::PoolError;
use crate::types::{ClaimStatus, MAX_APPROVAL_WINDOW_SECONDS, REVOCATION_TTL_LEDGERS};

const ENTITLEMENT: i128 = STROOPS_PER_UNIT;
const TIER_C: u32 = 3;

type SubmitResult = Result<Result<BytesN<32>, ConversionError>, Result<PoolError, InvokeError>>;

/// Asserts the call failed as an opaque host trap rather than a typed error.
///
/// `Err(Err(_))` is the trap shape; `Err(Ok(pool_error))` is the typed shape.
/// Requiring the former documents, executably, that a bad signature is NOT
/// reportable as a named error at SDK 27, the accepted, documented cost of
/// Blocker 3 in the D1 eng review.
fn assert_signature_trap(r: SubmitResult) {
    match r {
        Err(Err(_)) => {}
        Err(Ok(e)) => panic!(
            "expected an opaque host trap from ed25519_verify, got typed {:?}, \
             if verification became typed, update the D1 docs too",
            e
        ),
        Ok(_) => panic!("expected signature verification to fail, but the claim was accepted"),
    }
}

/// Submits with an explicitly supplied deadline + signature, bypassing the
/// `submit_claim_signed` convenience helper. Every test here needs to control
/// those two arguments directly: that is what is under test.
#[allow(clippy::too_many_arguments)]
fn submit_raw(
    s: &Setup,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: &i128,
    tier: &u32,
    hack_timestamp: &u64,
    deadline: &u64,
    signature: &BytesN<64>,
) -> SubmitResult {
    s.client.try_submit_claim(
        caller,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        deadline,
        signature,
    )
}

fn zero_sig(env: &Env) -> BytesN<64> {
    BytesN::from_array(env, &[0u8; 64])
}

// -----------------------------------------------------------------------
// Happy path
// -----------------------------------------------------------------------

#[test]
fn oracle_path_accepts_a_valid_signature() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    let claim_id = submit_raw(
        &s,
        &s.oracle,
        &staker,
        &txh,
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
        &sig,
    )
    .unwrap()
    .unwrap();

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::PendingTime);
    assert_eq!(claim.entitlement, ENTITLEMENT);
}

// -----------------------------------------------------------------------
// Payload tampering: one test per signed field.
//
// Each signs over one set of values and submits a DIFFERENT set, so the
// contract rebuilds a payload the signature does not cover. Together these
// prove every field is actually inside the signed message rather than merely
// passed alongside it: a field omitted from `build_approval_payload` would
// make its test pass the claim through.
// -----------------------------------------------------------------------

#[test]
fn tampered_wallet_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let (other, _b2) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &other, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    ));
}

#[test]
fn tampered_tx_hash_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(
        &env,
        &s,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
    );

    assert_signature_trap(submit_raw(
        &s,
        &s.oracle,
        &staker,
        &tx_hash(&env, 2),
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
        &sig,
    ));
}

#[test]
fn tampered_entitlement_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // The economically interesting tamper: inflate the payout. Still well
    // inside the Tier C cap, so this is rejected by the signature and not
    // incidentally by `EntitlementExceedsTierCap`.
    let inflated = ENTITLEMENT * 2;
    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &inflated, &TIER_C, &hack, &deadline, &sig,
    ));
}

#[test]
fn tampered_tier_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // Tier C -> Tier A would triple the coverage cap. V8 added tier to the
    // signed payload for exactly this reason (an earlier fix).
    let tier_a: u32 = 1;
    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &tier_a, &hack, &deadline, &sig,
    ));
}

#[test]
fn tampered_hack_timestamp_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // One second earlier: still a valid timestamp on its own terms (not in
    // the future, not before the stake), so only the signature can catch it.
    let shifted = hack - 1;
    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &shifted, &deadline, &sig,
    ));
}

#[test]
fn tampered_deadline_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // Extended, but still inside the max window, so it passes both deadline
    // checks and reaches the crypto, proving the deadline is signed, not
    // merely range-checked.
    let extended = deadline + 60;
    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &extended, &sig,
    ));
}

#[test]
fn signature_for_a_different_contract_instance_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    // A second pool, same admin/oracle key material, different address.
    let other_pool = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);

    // Signed against the OTHER pool's address. Without `address(this)` in the
    // payload (V8 `:417`), one signed approval would be replayable across
    // every SAFU pool deployed on the network.
    let sig = sign_approval(
        &env,
        &other_pool,
        &staker,
        &txh,
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
    );

    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    ));
}

// -----------------------------------------------------------------------
// Wrong key material
// -----------------------------------------------------------------------

#[test]
fn signature_from_a_different_key_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval_with(
        &env,
        &s,
        &other_signing_key(),
        &staker,
        &txh,
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
    );

    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    ));
}

#[test]
fn zero_signature_is_rejected_on_the_oracle_path() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);

    assert_signature_trap(submit_raw(
        &s,
        &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
        &default_deadline(&env),
        &zero_sig(&env),
    ));
}

#[test]
fn a_v8_domain_payload_cannot_verify_here() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);

    // Byte-for-byte the Soroban payload except for V8's domain string.
    // Cross-chain replay is already impossible (different curve, different
    // key), but the domain makes that a property of the message itself.
    let payload = env.as_contract(&s.contract_id, || {
        let mut buf: Bytes = Symbol::new(&env, "SAFU_CLAIM_APPROVAL").to_xdr(&env);
        buf.append(&env.current_contract_address().to_xdr(&env));
        buf.append(&env.ledger().network_id().to_xdr(&env));
        buf.append(&staker.clone().to_xdr(&env));
        buf.append(&txh.clone().to_xdr(&env));
        buf.append(&ENTITLEMENT.to_xdr(&env));
        buf.append(&TIER_C.to_xdr(&env));
        buf.append(&hack.to_xdr(&env));
        buf.append(&deadline.to_xdr(&env));
        buf
    });
    let sig = sign_bytes(&env, &oracle_signing_key(), &payload);

    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    ));
}

// -----------------------------------------------------------------------
// Deadline boundaries: typed errors, not traps.
// -----------------------------------------------------------------------

#[test]
fn deadline_exactly_at_now_is_accepted() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    // The check is `now > deadline`, so now == deadline is still open.
    let deadline = now_ts(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    assert!(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    )
    .is_ok());
}

#[test]
fn deadline_one_second_past_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = now_ts(&env) - 1;
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    assert_eq!(
        submit_raw(
            &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
        ),
        Err(Ok(PoolError::SignatureExpired))
    );
}

#[test]
fn deadline_at_the_max_window_is_accepted() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = now_ts(&env) + MAX_APPROVAL_WINDOW_SECONDS;
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    assert!(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    )
    .is_ok());
}

#[test]
fn deadline_one_second_beyond_the_max_window_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = now_ts(&env) + MAX_APPROVAL_WINDOW_SECONDS + 1;
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // Rejected even though the signature itself is perfectly valid, the
    // bound exists so the revocation TTL can provably outlive any deadline.
    assert_eq!(
        submit_raw(
            &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
        ),
        Err(Ok(PoolError::SignatureDeadlineTooFar))
    );
}

/// The relationship the const-assert in types.rs enforces at compile time,
/// restated as a test so it appears in the suite's coverage rather than only
/// in the build.
#[test]
fn revocation_ttl_outlives_the_longest_legal_deadline() {
    assert!(REVOCATION_TTL_LEDGERS as u64 >= MAX_APPROVAL_WINDOW_SECONDS);
}

// -----------------------------------------------------------------------
// Attestation key: absence and rotation
// -----------------------------------------------------------------------

#[test]
fn missing_oracle_pubkey_fails_closed_with_a_typed_error() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    // `initialize` always sets the pubkey, so this state is unreachable
    // through the public API: it is removed directly here to prove the
    // defence-in-depth branch fails CLOSED and reports a named error rather
    // than unwrap-trapping into something indistinguishable from a bad
    // signature.
    env.as_contract(&s.contract_id, || {
        env.storage()
            .instance()
            .remove(&crate::storage::DataKey::OraclePubKey);
    });

    assert_eq!(
        submit_raw(
            &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
        ),
        Err(Ok(PoolError::OraclePubKeyNotSet))
    );
}

#[test]
fn rotating_the_pubkey_invalidates_old_signatures_and_accepts_new_ones() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let new_key = other_signing_key();
    // Pre-audit hardening: the oracle moves only through governance (7 days).
    gov_apply(&env, &s, crate::GovChange::Oracle(s.oracle.clone(), verifying_key_bytes(&env, &new_key)));

    // Signed after the wait so the deadline is live: only the key differs.
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let old_sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);
    // The retired key traps: the rotation hazard, asserted so it stays a
    // known cost rather than a surprise.
    assert_signature_trap(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &old_sig,
    ));

    let new_sig = sign_approval_with(
        &env, &s, &new_key, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline,
    );
    assert!(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &new_sig,
    )
    .is_ok());
}

#[test]
fn rotating_the_oracle_address_with_the_same_key_keeps_verifying() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let new_oracle = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Oracle(new_oracle.clone(), verifying_key_bytes(&env, &s.oracle_key)));

    let claim_id = submit_claim_signed(
        &env,
        &s,
        &new_oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(
        s.client.get_claim(&claim_id).unwrap().status,
        ClaimStatus::PendingTime
    );
}

// -----------------------------------------------------------------------
// Oracle rotation is atomic: address and key move together, in one
// governance change, so they can never belong to different oracles.
// -----------------------------------------------------------------------

#[test]
fn atomic_rotation_swaps_both_identities_in_one_change() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let new_oracle = Address::generate(&env);
    let new_key = other_signing_key();
    gov_apply(&env, &s, crate::GovChange::Oracle(new_oracle.clone(), verifying_key_bytes(&env, &new_key)));

    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let old_sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);
    assert_signature_trap(submit_raw(
        &s, &new_oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &old_sig,
    ));
    let new_sig = sign_approval_with(
        &env, &s, &new_key, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline,
    );
    assert!(submit_raw(
        &s, &new_oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &new_sig,
    )
    .is_ok());
}

#[test]
fn rejected_oracle_rotation_leaves_both_identities_untouched() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);

    // A role cannot also be the oracle: refused at proposal, nothing written.
    let orphan_key = other_signing_key();
    assert_eq!(
        s.client.try_propose_change(
            &s.admin,
            &crate::GovChange::Oracle(s.co_signer.clone(), verifying_key_bytes(&env, &orphan_key)),
        ),
        Err(Ok(PoolError::GovRoleNotDistinct))
    );

    let claim_id = submit_claim_signed(
        &env,
        &s,
        &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(
        s.client.get_claim(&claim_id).unwrap().status,
        ClaimStatus::PendingTime
    );
}

// -----------------------------------------------------------------------
// Revocation
// -----------------------------------------------------------------------

#[test]
fn a_revoked_approval_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    s.client.revoke_approval(
        &s.admin, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline,
    );

    // Typed, not a trap: revocation is checked BEFORE `ed25519_verify`, so a
    // revoked-but-otherwise-valid approval reports why it was refused.
    assert_eq!(
        submit_raw(
            &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
        ),
        Err(Ok(PoolError::ApprovalRevoked))
    );
}

#[test]
fn revoking_one_approval_does_not_block_a_different_one() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);

    // Revoke tx 1 only. The revocation key is the payload hash, so it must
    // not collide with any other approval.
    s.client.revoke_approval(
        &s.admin,
        &staker,
        &tx_hash(&env, 1),
        &ENTITLEMENT,
        &TIER_C,
        &hack,
        &deadline,
    );

    let txh2 = tx_hash(&env, 2);
    let sig2 = sign_approval(&env, &s, &staker, &txh2, &ENTITLEMENT, &TIER_C, &hack, &deadline);
    assert!(submit_raw(
        &s, &s.oracle, &staker, &txh2, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig2,
    )
    .is_ok());
}

#[test]
fn revoke_approval_is_admin_only() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);

    // The oracle is explicitly NOT allowed to revoke its own approvals,
    // revocation exists to override the oracle, so oracle authority over it
    // would defeat the control.
    assert_eq!(
        s.client.try_revoke_approval(
            &s.oracle,
            &staker,
            &tx_hash(&env, 1),
            &ENTITLEMENT,
            &TIER_C,
            &now_ts(&env),
            &default_deadline(&env),
        ),
        Err(Ok(PoolError::CallerNotAdmin))
    );
}

#[test]
fn revoking_an_already_expired_approval_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);

    // Rejected rather than silently accepted: such an approval is already
    // dead on `SignatureExpired`, and a successful-looking revocation would
    // imply protection that is not being provided.
    assert_eq!(
        s.client.try_revoke_approval(
            &s.admin,
            &staker,
            &tx_hash(&env, 1),
            &ENTITLEMENT,
            &TIER_C,
            &now_ts(&env),
            &(now_ts(&env) - 1),
        ),
        Err(Ok(PoolError::SignatureExpired))
    );
}

#[test]
fn revoking_a_deadline_beyond_the_max_window_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);

    // Such an approval could never be submitted anyway, and accepting the
    // revocation would write an entry whose TTL is not guaranteed to cover
    // the deadline it names.
    assert_eq!(
        s.client.try_revoke_approval(
            &s.admin,
            &staker,
            &tx_hash(&env, 1),
            &ENTITLEMENT,
            &TIER_C,
            &now_ts(&env),
            &(now_ts(&env) + MAX_APPROVAL_WINDOW_SECONDS + 1),
        ),
        Err(Ok(PoolError::SignatureDeadlineTooFar))
    );
}

// -----------------------------------------------------------------------
// Pre-audit hardening: the admin's unsigned claim path is removed
// -----------------------------------------------------------------------

#[test]
fn admin_cannot_file_a_claim_even_with_a_valid_oracle_signature() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);
    assert_eq!(
        submit_raw(&s, &s.admin, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig),
        Err(Ok(PoolError::CallerNotOracleOrAdmin))
    );
    // And with no signature at all (the old manual path).
    assert_eq!(
        submit_raw(&s, &s.admin, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &0u64, &zero_sig(&env)),
        Err(Ok(PoolError::CallerNotOracleOrAdmin))
    );
}

// -----------------------------------------------------------------------
// Replay
// -----------------------------------------------------------------------

#[test]
fn replaying_a_valid_approval_hits_the_claim_id_guard_not_the_signature() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);
    let sig = sign_approval(&env, &s, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline);

    assert!(submit_raw(
        &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
    )
    .is_ok());

    // The same payload again, still perfectly signed and unexpired. It is
    // stopped by `compute_claim_id(wallet, tx_hash)` + `ClaimAlreadyExists`:
    // the claim id IS the nonce. The specific error matters: it shows
    // replay defence does not depend on the signature layer, which is why
    // the deadline and revocation list are defence in depth rather than the
    // primary control.
    //
    // The wallet's `active_claim_id` is set by the first submission, so the
    // one-wallet-one-claim guard fires first. Either way the point holds:
    // the rejection is a typed state guard, never a signature failure.
    assert_eq!(
        submit_raw(
            &s, &s.oracle, &staker, &txh, &ENTITLEMENT, &TIER_C, &hack, &deadline, &sig,
        ),
        Err(Ok(PoolError::ClaimAlreadyActiveForStake))
    );
}

// -----------------------------------------------------------------------
// Payload encoding
// -----------------------------------------------------------------------

/// Independently reconstructs the payload longhand instead of calling
/// `build_approval_payload`, so a reordered, dropped, or added field fails
/// here. Every other test in this file signs with the same builder the
/// contract verifies with, which makes them self-consistent by construction.
/// This is the test that pins the encoding itself.
///
/// It is also the on-chain half of the contract that `api/signer.py` has to
/// reproduce; the cross-language half is the KMS round-trip test on the
/// Python side.
#[test]
fn payload_encoding_is_stable_and_field_ordered() {
    let env = new_env();
    let s = setup(&env);
    let wallet = Address::generate(&env);
    let txh = tx_hash(&env, 42);
    let entitlement: i128 = 123_456;
    let tier: u32 = 2;
    let hack: u64 = now_ts(&env);
    let deadline: u64 = hack + 3_600;

    let (built, longhand) = env.as_contract(&s.contract_id, || {
        let built = crate::claim::build_approval_payload(
            &env, &wallet, &txh, entitlement, tier, hack, deadline,
        );
        let mut longhand: Bytes = Symbol::new(&env, "SAFU_CLAIM_APPROVAL_SOROBAN").to_xdr(&env);
        longhand.append(&env.current_contract_address().to_xdr(&env));
        longhand.append(&env.ledger().network_id().to_xdr(&env));
        longhand.append(&wallet.to_xdr(&env));
        longhand.append(&txh.clone().to_xdr(&env));
        longhand.append(&entitlement.to_xdr(&env));
        longhand.append(&tier.to_xdr(&env));
        longhand.append(&hack.to_xdr(&env));
        longhand.append(&deadline.to_xdr(&env));
        (built, longhand)
    });

    assert_eq!(built, longhand);
}

#[test]
fn payload_binds_to_the_deploying_contract() {
    let env = new_env();
    let a = setup(&env);
    let b = setup(&env);
    let wallet = Address::generate(&env);
    let txh = tx_hash(&env, 1);
    let hack = now_ts(&env);
    let deadline = default_deadline(&env);

    let pa = env.as_contract(&a.contract_id, || {
        crate::claim::build_approval_payload(&env, &wallet, &txh, ENTITLEMENT, TIER_C, hack, deadline)
    });
    let pb = env.as_contract(&b.contract_id, || {
        crate::claim::build_approval_payload(&env, &wallet, &txh, ENTITLEMENT, TIER_C, hack, deadline)
    });

    assert_ne!(pa, pb);
}
