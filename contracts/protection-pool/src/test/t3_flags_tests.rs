//! T3 (2026-08-24), admission-side retry queue, bidirectional liquidity
//! rebalancing, and the `total_staked` shortfall reconciliation. Split out
//! for the same reason `d2_vault_tests` was: new mechanic, not existing
//! pool mechanics. Reuses `d2_vault_tests`'s `MockVault`/`with_vault`/
//! `assert_invariant` rather than duplicating the mock.

#![cfg(test)]

use super::common::*;
use super::d2_vault_tests::{assert_invariant, with_vault};
use crate::error::PoolError;
use crate::types::{
    ClaimStatus, BPS_DENOMINATOR, CLAIM_WINDOW_SECONDS, MAX_DEPLOY_BPS,
    MAX_REBALANCE_SLIPPAGE_BPS,
};

const TIER_C: u32 = 3;

// Scenario parameters. These are choices a test makes, not copies of contract
// constants, so they are named once here and everything else derives from them
// or from the contract's own constants (`MAX_STAKE`, `MAX_DEPLOY_BPS`,
// `CLAIM_WINDOW_SECONDS`, `MAX_REBALANCE_SLIPPAGE_BPS`, ...).

/// The small first deposit that gives the pool a reference share price.
const SEED_DEPLOY: i128 = STROOPS_PER_UNIT;
/// A deploy ceiling of half the pool, used where 80% would not leave room.
const HALF_CEILING_BPS: i128 = 5_000;
/// Reference ratio for the tests that need a non-1:1 share price.
const SHARES_PER_ASSET: i128 = 5;
/// Utilisation high enough that the stress cap sits in its lowest band and
/// the solvency headroom is narrower than that cap.
const NEAR_FULL_UTILISATION_BPS: i128 = 9_800;
/// A vault loss the pool must tolerate / must refuse, relative to
/// `MAX_REBALANCE_SLIPPAGE_BPS`.
const WITHIN_TOLERANCE_LOSS_BPS: i128 = 200;
const TOLERATED_LOSS_BPS: i128 = 300;
const BEYOND_TOLERANCE_LOSS_BPS: i128 = 1_000;
const _: () = assert!(WITHIN_TOLERANCE_LOSS_BPS < MAX_REBALANCE_SLIPPAGE_BPS);
const _: () = assert!(TOLERATED_LOSS_BPS < MAX_REBALANCE_SLIPPAGE_BPS);
const _: () = assert!(BEYOND_TOLERANCE_LOSS_BPS > MAX_REBALANCE_SLIPPAGE_BPS);

/// The smallest entitlement the stress cap refuses right now, so the claim
/// queues instead of being admitted.
fn queuing_entitlement(_env: &soroban_sdk::Env, s: &Setup<'_>) -> i128 {
    expected_stress_cap(s) + 1
}

// -----------------------------------------------------------------------
// Admission-side queue: submit_claim -> Reserved -> release/expire.
// -----------------------------------------------------------------------

#[test]
fn queued_claim_releases_once_capacity_frees() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    // One unit over the stress cap at 0% utilisation, so it queues.
    let entitlement = queuing_entitlement(&env, &s);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
    // Queuing must not touch capacity accounting, it was never admitted.
    assert_eq!(s.client.get_total_allocated(), 0);

    // Grow the pool until the stress cap clears the queued entitlement.
    while expected_stress_cap(&s) < entitlement {
        staked_wallet(&env, &s);
    }

    s.client.try_release_queued_claim(&claim_id);
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_ne!(claim.status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_total_allocated(), entitlement);
    // Wallet's queue slot is freed on release.
    assert!(s.client.get_stake(&w1).unwrap().reserved_claim_id.is_none());
}

#[test]
fn try_release_queued_claim_stays_reserved_while_still_blocked() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let entitlement = queuing_entitlement(&env, &s);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    // Nothing about pool capacity changed, still blocked.
    let result = s.client.try_try_release_queued_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::QueueReleaseNotYetEligible)));
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Reserved);
}

#[test]
fn try_release_queued_claim_on_unknown_claim_fails() {
    let env = new_env();
    let s = setup(&env);
    let fake_id = tx_hash(&env, 99); // right shape, not a real claim id
    let result = s.client.try_try_release_queued_claim(&fake_id);
    assert_eq!(result, Err(Ok(PoolError::NoSuchQueuedClaim)));
}

#[test]
fn expire_queued_claim_before_window_fails() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let entitlement = queuing_entitlement(&env, &s);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    let result = s.client.try_expire_queued_claim(&claim_id);
    assert_eq!(result, Err(Ok(PoolError::QueueNotYetExpired)));
}

#[test]
fn expire_queued_claim_after_window_clears_the_wallet_slot() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let hack_ts = now_ts(&env);
    let entitlement = queuing_entitlement(&env, &s);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &hack_ts,
    );
    // Pin the boundary itself: the check is `now <= hack_ts + WINDOW`, so at
    // EXACTLY the window it must still refuse, and one second later it must
    // sweep. A `<=` -> `<` mutation flips only in that one-ledger gap.
    advance_ledgers(&env, (CLAIM_WINDOW_SECONDS / SECONDS_PER_LEDGER) as u32);
    let now = now_ts(&env);
    assert_eq!(now, hack_ts + CLAIM_WINDOW_SECONDS, "must sit exactly on the window");
    assert_eq!(
        s.client.try_expire_queued_claim(&claim_id),
        Err(Ok(PoolError::QueueNotYetExpired))
    );

    advance_ledgers(&env, 1); // now strictly past it
    s.client.expire_queued_claim(&claim_id);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.status, ClaimStatus::Expired);
    assert!(s.client.get_stake(&w1).unwrap().reserved_claim_id.is_none());
}

/// 2-of-2 override, both approvers.
fn do_override(
    s: &Setup<'_>,
    wallet: &soroban_sdk::Address,
    hash: &soroban_sdk::BytesN<32>,
    entitlement: i128,
    tier: u32,
) {
    s.client.approve_override(&s.admin, wallet, hash, &entitlement, &tier);
    s.client.approve_override(&s.co_signer, wallet, hash, &entitlement, &tier);
}

/// REGRESSION: found by the 2026-08-24 T3 audit pass, not by a test.
///
/// A `Reserved` claim deliberately does not set `active_claim_id`, so the
/// one-claim-per-wallet guard in `execute_override` (which only inspects
/// `active_claim_id`) does not see it. A 2-of-2 override could therefore
/// create a second, DIFFERENT live claim for a wallet that already had one
/// queued. Releasing the queued one afterwards would then have produced two
/// independently-payable claims against a single stake, the exact invariant
/// the 2026-07-22 "Bug 2 fix" closed for the override path.
#[test]
fn releasing_a_queued_claim_is_refused_when_the_wallet_has_another_active_claim() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let (_w2, _b2) = staked_wallet(&env, &s); // more backing so the override is solvent

    // Queue a claim. Two wallets are staked so the override below is solvent,
    // which raises the stress cap above what a single-wallet pool has, so the
    // entitlement is derived from the cap as it stands now. One unit over it
    // queues, and stays well inside tier C's ceiling on this stake.
    let queued_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &queuing_entitlement(&env, &s), &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(s.client.get_claim(&queued_id).unwrap().status, ClaimStatus::Reserved);
    assert!(s.client.get_stake(&w1).unwrap().active_claim_id.is_none());

    // A 2-of-2 override creates a DIFFERENT live claim for the same wallet.
    do_override(&s, &w1, &tx_hash(&env, 2), MIN_STAKE, TIER_C);
    let active_id = s.client.get_stake(&w1).unwrap().active_claim_id.unwrap();
    assert_ne!(active_id, queued_id);

    // Releasing the queued claim must now be refused outright.
    let result = s.client.try_try_release_queued_claim(&queued_id);
    assert_eq!(result, Err(Ok(PoolError::WalletHasDifferentActiveClaim)));
    // And it must not have been admitted: still Reserved, still no allocation
    // of its own beyond what the override legitimately took.
    assert_eq!(s.client.get_claim(&queued_id).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_stake(&w1).unwrap().active_claim_id.unwrap(), active_id);
}

/// REGRESSION: same audit pass. An override that takes over the wallet's OWN
/// queued claim_id must release the queue slot with it. Otherwise the record
/// goes Active while `reserved_claim_id` still points at it, and
/// `expire_queued_claim` can never clear that pointer (it requires status ==
/// Reserved), permanently blocking the wallet from ever queuing again.
#[test]
fn override_of_the_same_claim_id_clears_the_queue_slot() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let (_w2, _b2) = staked_wallet(&env, &s);

    let hash = tx_hash(&env, 1);
    let queued_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &hash, &queuing_entitlement(&env, &s), &TIER_C, &now_ts(&env),
    );
    assert_eq!(s.client.get_claim(&queued_id).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_stake(&w1).unwrap().reserved_claim_id, Some(queued_id.clone()));

    // Override the SAME wallet+tx_hash, so it resolves to the same claim_id.
    do_override(&s, &w1, &hash, MIN_STAKE, TIER_C);

    let stake = s.client.get_stake(&w1).unwrap();
    assert_eq!(stake.active_claim_id, Some(queued_id.clone()));
    // The queue slot must be released, not left dangling.
    assert_eq!(stake.reserved_claim_id, None);

    // `reserved_claim_id == None` above IS the proof the wallet is not
    // soft-locked: deliberately not re-asserted via a fresh submit_claim
    // attempt: the override forfeits the stake (`withdrawn = true`), and
    // that guard sits EARLIER in submit_claim's validation order than the
    // ClaimAlreadyQueued guard, so such an attempt short-circuits on
    // AlreadyWithdrawn and proves nothing about the queue slot.
}

/// Isolates the SOLVENCY arm of `submit_claim`'s queue decision from the
/// stress-cap arm: the gap that let `total_allocated + entitlement >
/// total_staked` be mutated to `-` with every test still passing.
///
/// Pre-T3 this was covered for free: solvency returned `Err(Insolvent)`
/// immediately, so mutating it produced a DIFFERENT error and the test failed.
/// T3 merged both arms into one `insolvent || stress_capped` with a single
/// outcome, so the distinction is only observable when exactly one arm fires.
///
/// The window is narrow and worth writing down. `+` -> `-` flips the result
/// only when `allocated + e > staked` AND `allocated - e <= staked`; requiring
/// the stress arm to stay silent (`e <= stress_cap`) also forces
/// `stress_cap > headroom`, i.e. utilisation above 97% (rate is 300bps there,
/// so headroom must be under 3% of the pool). Hence: `NEAR_FULL_UTILISATION_BPS`
/// allocated against MAX_STAKE staked, with the entitlement halfway between the
/// headroom and the stress cap.
#[test]
fn submit_claim_queues_on_solvency_alone_with_the_stress_cap_silent() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE); // 2 * MAX_STAKE staked

    // Drive utilisation to the near-full level: the override forfeits w1's
    // stake, leaving MAX_STAKE staked while reserving most of it.
    let allocated = bps_of(MAX_STAKE, NEAR_FULL_UTILISATION_BPS);
    do_override(&s, &w1, &tx_hash(&env, 1), allocated, TIER_C);
    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    assert_eq!(s.client.get_total_allocated(), allocated);

    // The stress cap at this utilisation is wider than the solvency headroom.
    // An entitlement between the two breaches solvency while sitting UNDER the
    // stress cap: so only the solvency arm can be responsible for queuing.
    let headroom = MAX_STAKE - allocated;
    let cap = expected_stress_cap(&s);
    assert!(headroom < cap, "the stress cap must be wider than the headroom");
    let entitlement = (headroom + cap) / 2;
    assert!(entitlement > headroom && entitlement <= cap);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w2, &tx_hash(&env, 2), &entitlement, &TIER_C, &now_ts(&env),
    );

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(
        claim.status,
        ClaimStatus::Reserved,
        "must queue on solvency alone, if this admits, the solvency arm is dead"
    );
    // Still not admitted: queuing reserves no capacity.
    assert_eq!(s.client.get_total_allocated(), allocated);

    // The SAME window applies to the release-side solvency re-check
    // (claim.rs try_release_queued_claim). Nothing has changed, so releasing
    // must be refused: and refused on solvency, with the stress arm still
    // silent. A `+` -> `-` there would compute 98 - 2.5 = 95.5, conclude the
    // pool is solvent, and wrongly admit.
    assert_eq!(
        s.client.try_try_release_queued_claim(&claim_id),
        Err(Ok(PoolError::QueueReleaseNotYetEligible))
    );
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_total_allocated(), allocated);
}

/// Both release-side comparisons are `>`, not `>=`: an entitlement that
/// EXACTLY fills the remaining solvency headroom AND exactly fills the day's
/// remaining stress cap must still be admitted.
///
/// Getting here needs care. A claim cannot simply be submitted at the
/// boundary: at exact fill `submit_claim` admits it outright, leaving no
/// queued state to release. It has to be queued while the pool is tighter,
/// then the pool GROWS into the boundary.
///
/// The numbers are chosen so both arms land on their line simultaneously: at
/// `EXACT_FILL_UTILISATION_BPS` the lowest stress band's cap equals the
/// solvency headroom. One test, both boundaries, and any `>` -> `>=` or
/// `>` -> `==` on either line turns this release into a refusal.
#[test]
fn queued_claim_releases_when_both_arms_sit_exactly_on_their_boundary() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let restored_staked = 2 * MAX_STAKE;

    // Reserve most of the restored pool via override, forfeits w1's stake, so
    // staked falls to MAX_STAKE while allocated stays at the reserved level.
    let allocated = bps_of(restored_staked, EXACT_FILL_UTILISATION_BPS);
    do_override(&s, &w1, &tx_hash(&env, 1), allocated, TIER_C);
    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    assert_eq!(s.client.get_total_allocated(), allocated);

    // The entitlement is exactly the headroom the pool will have once it is
    // restored. Against the shrunken pool it is deeply insolvent, so it queues.
    let entitlement = restored_staked - allocated;
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w2, &tx_hash(&env, 2), &entitlement, &TIER_C, &now_ts(&env),
    );
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);

    // A fresh staker restores the pool to its original size.
    staked_wallet_amount(&env, &s, MAX_STAKE);
    assert_eq!(s.client.get_total_staked(), restored_staked);
    // Both arms now sit exactly on their line:
    //   solvency: allocated + entitlement == staked     (exact fill)
    //   stress:   entitlement == stress cap             (exact fill)
    assert_eq!(allocated + entitlement, restored_staked);
    assert_eq!(expected_stress_cap(&s), entitlement);
    s.client.try_release_queued_claim(&claim_id);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_ne!(
        claim.status,
        ClaimStatus::Reserved,
        "exact fill on both arms must release under strict `>`"
    );
    assert_eq!(s.client.get_total_allocated(), restored_staked);
}

/// Release-side STRESS-CAP comparison is `>`, not `>=`: an entitlement that
/// exactly fills the day's remaining cap must be admitted. Solvency is kept
/// slack here so only the stress arm can be responsible.
#[test]
fn queued_claim_releases_at_the_exact_stress_cap_boundary() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);

    // At zero utilisation the cap is proportional to the pool, so an
    // entitlement of twice today's cap is exactly the cap of a doubled pool.
    // It exceeds today's cap but stays inside solvency, so it queues on the
    // stress arm alone.
    let entitlement = 2 * expected_stress_cap(&s);
    assert!(entitlement < MAX_STAKE, "must stay inside solvency");
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);

    // Doubling the pool puts the cap at exactly the entitlement.
    staked_wallet_amount(&env, &s, MAX_STAKE);
    assert_eq!(s.client.get_total_staked(), 2 * MAX_STAKE);
    assert_eq!(expected_stress_cap(&s), entitlement);

    // entitlement > cap is false under strict `>`, so it releases. `>=` refuses.
    s.client.try_release_queued_claim(&claim_id);
    assert_ne!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);
    assert_eq!(s.client.get_total_allocated(), entitlement);
}

/// `yield_balance`'s FIRST term (`liquid + deployed`). The companion test
/// above runs with nothing deployed, so that `+` was unobservable, with a
/// live vault position it is not.
#[test]
fn yield_balance_is_unaffected_by_deployment_location_or_claims() {
    // Rewritten 2026-09-18: this used to prove the residual formula correctly
    // counted deployed XLM as part of "yield". Under the explicit-counter
    // model, ProtocolYieldBalance has NO dependency on total_staked,
    // total_allocated or deployment location at all, it only ever moves via
    // extract_yield's split and withdraw_yield's debit. Proving it stays 0
    // through deployment AND a claim activation is the stronger, structural
    // version of the same property.
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let (_w2, _b2) = staked_wallet(&env, &s);
    with_vault(&env, &s, HALF_CEILING_BPS);

    let deployed = MID_STAKE / 2;
    s.client.deploy_to_vault(&deployed, &0);
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    assert_eq!(s.client.get_yield_balance(), 0);

    // A live claim forfeits w1's stake, the exact shape that produced the
    // old formula's live "+4,100 XLM phantom yield" bug. The explicit
    // counter cannot be affected by it: nothing here calls extract_yield.
    do_override(&s, &w1, &tx_hash(&env, 1), MID_STAKE / 2, TIER_C);
    assert_eq!(
        s.client.get_yield_balance(),
        0,
        "a claim forfeiture must never move protocol yield balance"
    );
    assert_invariant(&s);
}

// -----------------------------------------------------------------------
// total_allocated interaction: the condition every vault test was missing.
//
// Added 2026-08-24 after a cargo-mutants run (144 mutants, --in-diff scoped
// to the T3 changes) showed EVERY `total_allocated` term in the new code was
// mutable with zero test failures. Root cause was one mistake repeated: every
// vault test ran with no active claims, so `total_allocated == 0` and terms
// like `total_staked + total_allocated` / `liquid - total_allocated` were
// arithmetically identical under mutation. The functions were being tested
// with nothing owed: which is precisely the case they exist to handle.
// -----------------------------------------------------------------------

/// `get_yield_balance()` must subtract BOTH what is owed to stakers and what
/// is owed to already-approved claims. This is the live 2026-08-20 finding:
/// two activated claims made the getter read +4,100 XLM of "yield" that was
/// really the claimants' own forfeited principal.
///
/// The pre-existing coverage in `d2_vault_tests` all runs at
/// `total_allocated == 0`, where the T3 fix is a mathematical no-op, so the
/// fix shipped with no test exercising the case it was written for.
#[test]
fn yield_balance_excludes_principal_owed_to_active_claims() {
    // Rewritten 2026-09-18: the pre-fix bug this test documented (a
    // forfeiture misread as +4,100 XLM "yield") is now structurally
    // impossible rather than merely corrected, ProtocolYieldBalance never
    // derives from total_staked/total_allocated arithmetic at all. Kept as
    // a regression test proving that holds across repeated claim activity.
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE); // 2 * MAX_STAKE staked

    assert_eq!(s.client.get_yield_balance(), 0);

    // Activate a claim via override, forfeits w1's stake (total_staked
    // falls by MAX_STAKE) while the asset stays in the pool, now earmarked as
    // total_allocated. This is exactly the shape that produced the live
    // mislabel under the old residual formula.
    let entitlement = bps_of(MAX_STAKE, 4_000);
    do_override(&s, &w1, &tx_hash(&env, 1), entitlement, TIER_C);

    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert_eq!(s.client.get_liquid_balance(), 2 * MAX_STAKE);
    assert_eq!(
        s.client.get_yield_balance(),
        0,
        "a forfeiture must never be misread as protocol yield"
    );

    // A second live claim must not move it either, proving this isn't a
    // coincidence of the first claim's numbers.
    let (w3, _b3) = staked_wallet_amount(&env, &s, MAX_STAKE); // +MAX_STAKE staked and liquid
    let second_entitlement = bps_of(MAX_STAKE, 9_500);
    do_override(&s, &w3, &tx_hash(&env, 2), second_entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement + second_entitlement);
    assert_eq!(s.client.get_yield_balance(), 0);
    assert_invariant(&s);
}

/// `auto_deploy_liquidity` must treat claim-reserved asset as untouchable.
/// Every prior test ran with `total_allocated == 0`, so `liquid -
/// total_allocated` was indistinguishable from `liquid + total_allocated`.
#[test]
fn auto_deploy_liquidity_will_not_deploy_xlm_owed_to_a_live_claim() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let (_w2, _b2) = staked_wallet(&env, &s); // 2 * MID_STAKE staked
    with_vault(&env, &s, MAX_DEPLOY_BPS); // the highest ceiling, deliberately generous

    // Bootstrap a reference rate with a small manual deposit.
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);

    // Reserve a large entitlement so total_allocated dominates.
    let entitlement = bps_of(MID_STAKE, 7_500);
    do_override(&s, &w1, &tx_hash(&env, 1), entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);

    let liquid_before = s.client.get_liquid_balance();
    let deployed_before = s.client.get_total_deployed_asset();

    s.client.auto_deploy_liquidity();

    // The invariant that matters: liquid must never fall below what is owed
    // to live claims, no matter how much ceiling headroom exists.
    let liquid_after = s.client.get_liquid_balance();
    assert!(
        liquid_after >= entitlement,
        "deployed into claim-reserved asset: liquid {} < allocated {}",
        liquid_after, entitlement
    );
    // It deployed exactly min(idle, ceiling room), pins BOTH bounds rather
    // than asserting a direction. The ceiling binds here: total_staked fell
    // by MID_STAKE when the override forfeited w1's stake, so the ceiling is
    // MAX_DEPLOY_BPS of what is left, against SEED_DEPLOY already deployed.
    let idle = liquid_before - entitlement;
    let ceiling = bps_of(s.client.get_total_staked(), MAX_DEPLOY_BPS);
    let room = ceiling - deployed_before;
    let deployed_delta = s.client.get_total_deployed_asset() - deployed_before;
    assert_eq!(deployed_delta, idle.min(room));
    assert!(room < idle, "ceiling should be the binding constraint here");
    assert_invariant(&s);
}

/// `ensure_liquidity`'s claims-shortfall arm, pinned at the exact boundary.
/// `queued_claim_releases_once_capacity_frees` and friends leave solvency
/// wildly clear, so `>` vs `>=` at these comparisons was never probed.
#[test]
fn ensure_liquidity_pulls_exactly_to_the_allocated_line_no_further() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, MAX_DEPLOY_BPS);

    // Deploy modestly, then reserve a claim larger than what stays liquid.
    // Deploying 60% of MAX_STAKE leaves the post-override ratio (against the
    // MAX_STAKE left staked, at the MAX_DEPLOY_BPS ceiling) INSIDE the line, so `over_ceiling` is zero and this
    // isolates the claims-shortfall arm: otherwise the ceiling arm dominates
    // and the assertion below would be measuring the wrong thing.
    s.client.deploy_to_vault(&bps_of(MAX_STAKE, 6_000), &0);
    let entitlement = bps_of(MAX_STAKE, 15_000);
    do_override(&s, &w1, &tx_hash(&env, 1), entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert!(s.client.get_liquid_balance() < entitlement);
    let ceiling = bps_of(s.client.get_total_staked(), MAX_DEPLOY_BPS);
    assert!(s.client.get_total_deployed_asset() <= ceiling, "ceiling arm must be idle");

    let deployed_before = s.client.get_total_deployed_asset();
    s.client.ensure_liquidity();

    // Exactly the line, not past it: pulling more would be needless vault
    // churn, pulling less would leave the claim unpayable.
    assert_eq!(s.client.get_liquid_balance(), entitlement);
    assert!(s.client.get_total_deployed_asset() < deployed_before);
    assert_invariant(&s);
}

/// The slippage floor must actually REJECT when the vault mints materially
/// fewer shares than the contract's own reference rate predicts.
///
/// This is the test the 1:1 mock could never express. `min_shares_out` is a
/// floor, and with a 1:1 rate the mock always delivered exactly the expected
/// count, so every mutation of the share-price arithmetic merely lowered a
/// floor that was being cleared anyway. Minting 10% below the reference rate,
/// beyond MAX_REBALANCE_SLIPPAGE_BPS, makes the floor load-bearing:
/// a mutated formula computes a nonsense expectation, fails to reject, and
/// this assertion catches it.
#[test]
fn auto_deploy_liquidity_rejects_a_deposit_below_the_slippage_floor() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);

    // Bootstrap at 1:1, establishing the reference rate the contract will use.
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);
    assert_eq!(s.client.get_total_deployed_shares(), SEED_DEPLOY);
    assert_eq!(s.client.get_total_deployed_asset(), SEED_DEPLOY);

    // Vault now mints fewer shares per unit of asset than that reference,
    // worse than the MAX_REBALANCE_SLIPPAGE_BPS the contract tolerates.
    mock.set_deposit_rate_bps(&(BPS_DENOMINATOR - BEYOND_TOLERANCE_LOSS_BPS));

    assert_eq!(
        s.client.try_auto_deploy_liquidity(),
        Err(Ok(PoolError::MinSharesNotMet))
    );
    // Rejected cleanly: nothing moved.
    assert_eq!(s.client.get_total_deployed_asset(), SEED_DEPLOY);
    assert_invariant(&s);
}

/// A deposit inside the tolerance still succeeds, and the contract records the
/// shares the vault ACTUALLY minted rather than the count it predicted.
#[test]
fn auto_deploy_liquidity_accepts_within_tolerance_and_records_real_shares() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);

    s.client.deploy_to_vault(&SEED_DEPLOY, &0);
    // A small loss below the reference, inside MAX_REBALANCE_SLIPPAGE_BPS.
    let minted_rate_bps = BPS_DENOMINATOR - WITHIN_TOLERANCE_LOSS_BPS;
    mock.set_deposit_rate_bps(&minted_rate_bps);

    let shares_before = s.client.get_total_deployed_shares();
    let xlm_before = s.client.get_total_deployed_asset();
    s.client.auto_deploy_liquidity();

    let xlm_delta = s.client.get_total_deployed_asset() - xlm_before;
    let shares_delta = s.client.get_total_deployed_shares() - shares_before;
    assert!(xlm_delta > 0, "should have deployed something");
    // Shares tracked at what was minted, not at the 1:1 amount.
    assert_eq!(shares_delta, bps_of(xlm_delta, minted_rate_bps));
    assert_invariant(&s);
}

/// `try_release_queued_claim`'s solvency re-check is `>`, not `>=`: an
/// entitlement that EXACTLY fills the remaining solvency headroom must be
/// admitted. Prior release tests left solvency wildly clear, so the boundary
/// itself was never pinned and `>` -> `>=` / `==` survived mutation.
#[test]
fn queued_claim_releases_when_entitlement_exactly_fills_solvency_headroom() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE);

    // Queue something the day-1 stress cap refuses, with room to spare.
    let entitlement = queuing_entitlement(&env, &s) + bps_of(expected_stress_cap(&s), 2_000);
    let claim_id = submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    assert_eq!(s.client.get_claim(&claim_id).unwrap().status, ClaimStatus::Reserved);

    // Grow the pool until BOTH arms clear, then release.
    while expected_stress_cap(&s) < entitlement {
        staked_wallet_amount(&env, &s, MAX_STAKE);
    }
    s.client.try_release_queued_claim(&claim_id);

    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_ne!(claim.status, ClaimStatus::Reserved);
    // Admitted exactly once, for exactly the pinned entitlement.
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert_eq!(claim.entitlement, entitlement);
}

// -----------------------------------------------------------------------
// Bidirectional liquidity rebalancing: ensure_liquidity (pull) /
// auto_deploy_liquidity (push), and the shared total_staked shortfall fix.
// -----------------------------------------------------------------------

#[test]
fn ensure_liquidity_pulls_exactly_the_shortfall() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    let deployed = bps_of(MAX_STAKE, MAX_DEPLOY_BPS); // right up to the ceiling
    s.client.deploy_to_vault(&deployed, &0);

    // The entitlement exceeds current liquid by a small margin but is within
    // the tier cap and the stress cap.
    let entitlement = s.client.get_liquid_balance() + MAX_STAKE / 100;
    assert!(entitlement <= expected_stress_cap(&s));
    submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert!(s.client.get_liquid_balance() < entitlement);

    let shortfall = entitlement - s.client.get_liquid_balance();
    s.client.ensure_liquidity();

    assert_eq!(s.client.get_liquid_balance(), entitlement);
    assert_eq!(s.client.get_total_deployed_asset(), deployed - shortfall);
    assert_invariant(&s);
}

/// Case found 2026-08-24: stakers withdrawing shrinks `total_staked`
/// while `deployed_asset` is unchanged, so the vault's SHARE of the pool climbs
/// above `deploy_bps` without a single new deployment. Previously documented
/// as drift needing an admin `provide_liquidity`; `ensure_liquidity` now
/// corrects it, making `deploy_bps` a genuine two-way line.
#[test]
fn ensure_liquidity_pulls_back_when_withdrawals_push_the_ratio_over_the_ceiling() {
    let env = new_env();
    let s = setup(&env);
    let (w1, b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE); // 2 * MAX_STAKE staked
    with_vault(&env, &s, HALF_CEILING_BPS);

    // Deploy right up to the ceiling.
    s.client.deploy_to_vault(&bps_of(2 * MAX_STAKE, HALF_CEILING_BPS), &0);
    assert_eq!(s.client.get_deployment_ratio_bps(), HALF_CEILING_BPS);

    // w1 withdraws. total_staked halves while deployed_asset is unchanged, so
    // the ratio doubles: far above the configured line, with no new
    // deployment having occurred.
    s.client.withdraw(&w1, &b1);
    assert_eq!(s.client.get_total_staked(), MAX_STAKE);
    assert!(s.client.get_deployment_ratio_bps() > HALF_CEILING_BPS);

    // No claims exist, so the claims-shortfall arm is zero. Pre-fix this
    // returned Ok(0) and left the drift in place.
    assert_eq!(s.client.get_total_allocated(), 0);
    s.client.ensure_liquidity();

    // Back EXACTLY on the configured line, self-corrected. Asserting the
    // precise figure rather than `<= ceiling`: a loose bound would let the
    // over-ceiling arithmetic be mutated (pull too much / too little) while
    // still landing somewhere under the line.
    let ceiling = bps_of(s.client.get_total_staked(), HALF_CEILING_BPS);
    assert_eq!(s.client.get_total_deployed_asset(), ceiling);
    assert_eq!(s.client.get_deployment_ratio_bps(), HALF_CEILING_BPS);
    assert_invariant(&s);
}

#[test]
fn ensure_liquidity_is_a_noop_when_nothing_is_short() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    // Nothing deployed, nothing allocated: liquid comfortably covers
    // total_allocated (0).
    assert_eq!(s.client.ensure_liquidity(), 0);
    assert_eq!(s.client.get_total_deployed_shares(), 0);
}

#[test]
fn ensure_liquidity_refuses_a_loss_beyond_the_slippage_bound() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);
    let deployed = bps_of(MAX_STAKE, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&deployed, &0);

    let entitlement = s.client.get_liquid_balance() + MAX_STAKE / 100;
    submit_claim_signed(
        &env, &s, &s.oracle, &w1, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    assert!(s.client.get_liquid_balance() < entitlement);

    // A loss beyond the MAX_REBALANCE_SLIPPAGE_BPS bound.
    mock.set_rate_bps(&(BPS_DENOMINATOR - BEYOND_TOLERANCE_LOSS_BPS));
    // The real vault refuses this itself; turn that off to prove the pool's
    // own floor check (defence in depth).
    mock.disable_min_check();

    let result = s.client.try_ensure_liquidity();
    assert_eq!(result, Err(Ok(PoolError::MinAmountNotMet)));
    // Redeem reverts entirely on the floor, nothing partially happened.
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
}

#[test]
fn redeem_shortfall_within_tolerance_marks_total_staked_down() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);
    let deployed = bps_of(MAX_STAKE, HALF_CEILING_BPS);
    s.client.deploy_to_vault(&deployed, &0);

    // A real Blend loss: within MAX_REBALANCE_SLIPPAGE_BPS, so this succeeds.
    let rate_bps = BPS_DENOMINATOR - TOLERATED_LOSS_BPS;
    mock.set_rate_bps(&rate_bps);

    let shares = s.client.get_total_deployed_shares(); // 1:1 mint
    assert_eq!(shares, deployed);
    let staked_before = s.client.get_total_staked();
    s.client.provide_liquidity(&shares, &0);

    let asset_received = bps_of(shares, rate_bps);
    let expected_shortfall = shares - asset_received;
    assert_eq!(s.client.get_total_staked(), staked_before - expected_shortfall);
    assert_invariant(&s);
}

#[test]
fn auto_deploy_liquidity_requires_a_prior_manual_deposit() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    // No deploy_to_vault call ever made, no reference rate to bound
    // a deposit's slippage against yet.
    let result = s.client.try_auto_deploy_liquidity();
    assert_eq!(result, Err(Ok(PoolError::NothingDeployed)));
}

#[test]
fn auto_deploy_liquidity_pushes_idle_cash_up_to_the_ceiling() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, HALF_CEILING_BPS);

    // Bootstrap: one manual deposit gives auto_deploy_liquidity a
    // reference rate to check its own deposit against.
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);

    // v1: a new stake would push in-path, so call the public function on the
    // idle cash already there (the stake above predates the seed deposit).
    let idle_before = s.client.get_liquid_balance() - s.client.get_total_allocated();
    let ceiling = bps_of(s.client.get_total_staked(), HALF_CEILING_BPS);
    let room = ceiling - s.client.get_total_deployed_asset();
    let expected_deploy = idle_before.min(room);

    s.client.auto_deploy_liquidity();

    assert_eq!(s.client.get_total_deployed_asset(), SEED_DEPLOY + expected_deploy);
    assert_invariant(&s);
}

#[test]
fn a_new_stake_pushes_idle_cash_up_to_the_ceiling_itself() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, HALF_CEILING_BPS);
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);

    // More stake arrives: the stake itself pushes idle cash to the line.
    staked_wallet_amount(&env, &s, MAX_STAKE);

    let ceiling = bps_of(s.client.get_total_staked(), HALF_CEILING_BPS);
    assert_eq!(s.client.get_total_deployed_asset(), ceiling);
    // Nothing left for the public function to do.
    assert_eq!(s.client.auto_deploy_liquidity(), 0);
    assert_invariant(&s);
}

// -----------------------------------------------------------------------
// Mutation-gap closures: added 2026-08-24 after the full 144-mutant
// campaign (134 caught / 9 missed / 1 unviable). Six of the nine misses
// were real gaps, all in the liquidity-rebalancing pair. Root cause of the
// skew: the audit pass hand-wrote seven tests for `ensure_liquidity` and
// never gave `auto_deploy_liquidity` the same treatment.
//
// The remaining three misses are provably equivalent and are documented as
// exclusions in `.cargo/mutants.toml` instead.
// -----------------------------------------------------------------------

/// Kills `vault.rs:542` (`liquid - total_allocated` -> `+`) and BOTH
/// `vault.rs:563:24` mutants (`<` -> `==`, `<` -> `<=`).
///
/// `auto_deploy_liquidity_will_not_deploy_xlm_owed_to_a_live_claim` asserts
/// `room < idle`: the CEILING binds there, so `amount = room` and `idle`'s
/// value never reaches the outcome. That made the `-`/`+` mutation on idle
/// invisible, and kept `liquid - amount` strictly ABOVE `total_allocated`,
/// so the `==`/`<=` mutants on the allocation guard never fired either.
///
/// Here idle binds instead. The trick: reserve MORE than the stake the
/// override forfeits. Forfeiture shrinks `total_staked` (and with it the
/// ceiling) while the asset itself stays in the pool, so a large enough
/// entitlement pushes idle below ceiling room.
#[test]
fn auto_deploy_liquidity_deploys_exactly_idle_when_idle_is_the_binding_constraint() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let (_w2, _b2) = staked_wallet_amount(&env, &s, MAX_STAKE); // 2 * MAX_STAKE staked
    with_vault(&env, &s, MAX_DEPLOY_BPS); // the ceiling must NOT bind

    s.client.deploy_to_vault(&SEED_DEPLOY, &0); // 1:1 reference rate

    let entitlement = bps_of(MAX_STAKE, 15_000); // > the MAX_STAKE w1 forfeits
    do_override(&s, &w1, &tx_hash(&env, 1), entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);

    let liquid_before = s.client.get_liquid_balance();
    let deployed_before = s.client.get_total_deployed_asset();
    let idle = liquid_before - entitlement;
    let room = bps_of(s.client.get_total_staked(), MAX_DEPLOY_BPS) - deployed_before;
    assert!(
        idle < room,
        "idle must be the binding constraint here: idle {} room {}",
        idle, room
    );

    s.client.auto_deploy_liquidity();

    // Exact, not an inequality: an inequality is what let the `+` mutant live.
    assert_eq!(s.client.get_total_deployed_asset() - deployed_before, idle);
    // Deploying exactly idle lands liquid precisely ON total_allocated, which
    // is the state that separates `<` from `==`/`<=` at the allocation guard.
    assert_eq!(s.client.get_liquid_balance(), entitlement);
    assert_invariant(&s);
}

/// Kills `vault.rs:569:34` (`amount * deployed_shares` -> `+`).
///
/// The mutation parses as `amount + (deployed_shares / deployed_asset)`, a
/// SUM, not a re-grouped quotient. With the 1:1 bootstrap every existing
/// test uses, `deployed_shares / deployed_asset` is 1, so the mutant computes
/// `amount + 1` and `min_shares_out` moves by less than one stroop, which
/// integer truncation erases entirely. That is why even the existing
/// below-the-floor test at a 9_000 rate could not kill it.
///
/// Making the reference RATIO load-bearing (`SHARES_PER_ASSET` shares per
/// unit, not 1) is what separates them: the original expects
/// `SHARES_PER_ASSET * amount`, the mutant expects `amount + SHARES_PER_ASSET`.
#[test]
fn auto_deploy_liquidity_rejects_when_share_price_collapses_against_the_reference() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);

    // Bootstrap at SHARES_PER_ASSET shares per unit, this ratio becomes the reference.
    mock.set_deposit_rate_bps(&(SHARES_PER_ASSET * BPS_DENOMINATOR));
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);
    assert_eq!(s.client.get_total_deployed_shares(), SHARES_PER_ASSET * SEED_DEPLOY);
    assert_eq!(s.client.get_total_deployed_asset(), SEED_DEPLOY);

    // Vault now mints 1 share per unit, far below the reference, far beyond
    // the MAX_REBALANCE_SLIPPAGE_BPS the contract tolerates.
    mock.set_deposit_rate_bps(&BPS_DENOMINATOR);

    assert_eq!(
        s.client.try_auto_deploy_liquidity(),
        Err(Ok(PoolError::MinSharesNotMet))
    );
    // Rejected cleanly: nothing moved.
    assert_eq!(s.client.get_total_deployed_asset(), SEED_DEPLOY);
    assert_invariant(&s);
}

/// Kills `vault.rs:591:22` (`shares_gained < min_shares_out` -> `<=`).
///
/// Minting at exactly MAX_REBALANCE_SLIPPAGE_BPS below the reference makes
/// `shares_gained` land precisely ON `min_shares_out`, both sides evaluate
/// the identical `amount * 9_500 / 10_000` expression, so they are equal
/// regardless of divisibility. `<` must ACCEPT that; `<=` rejects it.
#[test]
fn auto_deploy_liquidity_accepts_shares_exactly_at_the_slippage_floor() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);

    s.client.deploy_to_vault(&SEED_DEPLOY, &0); // 1:1 reference

    // Exactly on the tolerance line.
    let minted_rate_bps = BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS;
    mock.set_deposit_rate_bps(&minted_rate_bps);

    let shares_before = s.client.get_total_deployed_shares();
    let xlm_before = s.client.get_total_deployed_asset();

    s.client.auto_deploy_liquidity(); // must NOT panic, the floor is inclusive

    let xlm_delta = s.client.get_total_deployed_asset() - xlm_before;
    let shares_delta = s.client.get_total_deployed_shares() - shares_before;
    assert!(xlm_delta > 0, "should have deployed something");
    assert_eq!(shares_delta, bps_of(xlm_delta, minted_rate_bps));
    assert_invariant(&s);
}

/// Kills `vault.rs:780:36`: the `*` in
/// `min_asset_out = expected_asset * (BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS)
/// / BPS_DENOMINATOR`.
///
/// The mutation parses as `expected_asset + ((10_000 - 500) / 10_000)`, and
/// that quotient truncates to 0, so the mutant's floor is `expected_asset`
/// ITSELF. In other words it silently demands 100% of the expected proceeds
/// where the contract means to tolerate a 5% shortfall.
///
/// Killing it needs a redeem landing strictly INSIDE that 5% band: at or
/// above `0.95 * expected_asset` (original accepts) but below `expected_asset`
/// (mutant rejects). Any healthier redeem clears both floors and the
/// mutation is invisible: which is exactly how the first version of this
/// test, returning 250% of expected, let it survive.
///
/// A partial redeem against a non-1:1 reference sets the band up: with
/// `SHARES_PER_ASSET` shares per unit, expected_asset is the shortfall, the
/// original floor is `(BPS - MAX_REBALANCE_SLIPPAGE_BPS)` of it and the
/// mutant's is all of it. A withdraw rate `TOLERATED_LOSS_BPS` under the
/// reference returns an amount inside the band.
#[test]
fn ensure_liquidity_pulls_a_partial_tranche_priced_off_the_reference_ratio() {
    let env = new_env();
    let s = setup(&env);
    let (w1, _b1) = staked_wallet(&env, &s);
    let (_w2, _b2) = staked_wallet(&env, &s); // 2 * MID_STAKE staked
    let (_vault_id, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);

    // SHARES_PER_ASSET shares per unit reference.
    mock.set_deposit_rate_bps(&(SHARES_PER_ASSET * BPS_DENOMINATOR));
    s.client.deploy_to_vault(&SEED_DEPLOY, &0);
    assert_eq!(s.client.get_total_deployed_shares(), SHARES_PER_ASSET * SEED_DEPLOY);

    // Reserve just past liquid so the CLAIMS arm drives a small shortfall: a
    // fifth of the deployed position. Forfeiting w1 leaves MID_STAKE staked,
    // so the ceiling still sits above deployed_asset, the over-ceiling arm
    // stays at 0 and does not mask the shortfall we are pinning.
    let shortfall = SEED_DEPLOY / SHARES_PER_ASSET;
    let entitlement = s.client.get_liquid_balance() + shortfall;
    do_override(&s, &w1, &tx_hash(&env, 1), entitlement, TIER_C);
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert!(bps_of(MID_STAKE, MAX_DEPLOY_BPS) > SEED_DEPLOY, "ceiling arm must be idle");

    // The pull is partial: shares_needed = shortfall * shares / asset, against
    // the whole deployed position. expected_asset is therefore the shortfall.
    let shares_needed = shortfall * SHARES_PER_ASSET;
    assert!(shares_needed < SHARES_PER_ASSET * SEED_DEPLOY);
    let expected_asset = shortfall;
    let real_floor = bps_of(expected_asset, BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS);

    // Position came back a little light, but inside the tolerance: the rate is
    // the reference asset-per-share rate, less TOLERATED_LOSS_BPS.
    let reference_rate_bps = BPS_DENOMINATOR / SHARES_PER_ASSET;
    let light_rate_bps = bps_of(reference_rate_bps, BPS_DENOMINATOR - TOLERATED_LOSS_BPS);
    mock.set_rate_bps(&light_rate_bps);
    let asset_received = bps_of(shares_needed, light_rate_bps);
    // Inside the band: clears the real floor, fails the mutant's (which
    // demands 100% of expected_asset).
    assert!(asset_received >= real_floor && asset_received < expected_asset);

    let received = s.client.ensure_liquidity();
    assert_eq!(received, asset_received);
    assert_invariant(&s);
}
