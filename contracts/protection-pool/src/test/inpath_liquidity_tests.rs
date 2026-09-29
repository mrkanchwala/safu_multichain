//! v1 (2026-09-24): in-path two-way rebalancing (vault.rs rule 3).
//!
//! Money out (`withdraw`, `claim_stream`, `emergency_exit`,
//! `complete_backer_withdrawal`) pulls from the vault when cash is short;
//! money in (`stake`, `back`) pushes idle cash above the buffer.
//!
//! The second half stress-tests each exit path adversarially (
//! 2026-09-24): can anyone game the pull or push to take more than their
//! own money, strand someone else's, or break the accounting?

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::TokenClient;
use soroban_sdk::Address;

use super::common::*;
use super::d2_vault_tests::{assert_invariant, with_vault};
use crate::error::PoolError;
use crate::types::{AUTO_PUSH_MIN_BPS, MAX_DEPLOY_BPS};

const TIER_C: u32 = 3;
const BPS: i128 = 10_000;

/// Two MAX stakers, vault at 80%, first (manual) deposit made so the push
/// has a reference rate. Returns the stakers and the mock.
fn pool_at_ceiling<'a>(
    env: &'a soroban_sdk::Env,
    s: &Setup<'a>,
) -> ((Address, Address), (Address, Address), super::d2_vault_tests::MockVaultClient<'a>) {
    let a = staked_wallet_amount(env, s, MAX_STAKE);
    let b = staked_wallet_amount(env, s, MAX_STAKE);
    let (_v, mock) = with_vault(env, s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&bps_of(2 * MAX_STAKE, MAX_DEPLOY_BPS), &0);
    (a, b, mock)
}

fn balance(env: &soroban_sdk::Env, s: &Setup<'_>, who: &Address) -> i128 {
    TokenClient::new(env, &s.token_id).balance(who)
}

// -----------------------------------------------------------------------
// Pull
// -----------------------------------------------------------------------

#[test]
fn a_pull_refills_the_buffer_so_the_next_payment_needs_none() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, _m) = pool_at_ceiling(&env, &s);
    staked_wallet_amount(&env, &s, MAX_STAKE); // pushes: back to 80%
    let small = staked_wallet_amount(&env, &s, MIN_STAKE);

    s.client.withdraw(&a.0, &a.1);
    // After the pull the pool holds about the 20% buffer, not just the payment.
    let capacity = s.client.get_capacity();
    let buffer = bps_of(capacity, BPS - MAX_DEPLOY_BPS);
    assert!(s.client.get_liquid_balance() >= buffer);

    // A payment smaller than the buffer pays from cash: the vault doesn't move.
    let deployed = s.client.get_total_deployed_asset();
    s.client.withdraw(&small.0, &small.1);
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    assert_invariant(&s);
}

#[test]
fn enough_cash_means_no_vault_call() {
    let env = new_env();
    let s = setup(&env);
    let a = staked_wallet_amount(&env, &s, MAX_STAKE);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&SMALL, &0);
    let shares = s.client.get_total_deployed_shares();
    s.client.withdraw(&a.0, &a.1);
    assert_eq!(s.client.get_total_deployed_shares(), shares);
}

const SMALL: i128 = STROOPS_PER_UNIT;

#[test]
fn no_vault_keeps_the_old_error() {
    let env = new_env();
    let s = setup(&env);
    let a = staked_wallet_amount(&env, &s, MAX_STAKE);
    // Take cash out from under the pool (simulates a shortfall with no vault).
    let sink = Address::generate(&env);
    TokenClient::new(&env, &s.token_id).transfer(&s.contract_id, &sink, &1);
    assert_eq!(
        s.client.try_withdraw(&a.0, &a.1),
        Err(Ok(PoolError::InsufficientLiquidity))
    );
}

#[test]
fn a_dry_strategy_still_pays_the_shortfall_without_the_refill() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, mock) = pool_at_ceiling(&env, &s);
    // Blend can only return a little: enough for this payment, not the refill.
    let liquid = s.client.get_liquid_balance();
    mock.set_max_withdraw_shares(&(MAX_STAKE - liquid + 1));
    s.client.withdraw(&a.0, &a.1);
    assert_invariant(&s);
}

#[test]
fn a_loss_inside_the_floor_pays_and_is_marked_down() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, mock) = pool_at_ceiling(&env, &s);
    mock.set_rate_bps(&(BPS - 400)); // 4% loss, inside the 5% floor
    let before = balance(&env, &s, &a.1);
    s.client.withdraw(&a.0, &a.1);
    assert_eq!(balance(&env, &s, &a.1) - before, MAX_STAKE);
    // The realised loss is carried pool-wide, never pushed onto one staker.
    assert!(s.client.get_total_staked() < MAX_STAKE);
}

#[test]
fn a_loss_beyond_the_floor_is_refused_not_dumped() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, mock) = pool_at_ceiling(&env, &s);
    let deployed = s.client.get_total_deployed_asset();
    mock.set_rate_bps(&(BPS - 600)); // 6% loss
    assert_eq!(
        s.client.try_withdraw(&a.0, &a.1),
        Err(Ok(PoolError::InsufficientLiquidity))
    );
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
}

#[test]
fn rounding_never_leaves_a_payment_short() {
    let env = new_env();
    let s = setup(&env);
    let a = staked_wallet_amount(&env, &s, MAX_STAKE - 3);
    staked_wallet_amount(&env, &s, MAX_STAKE - 7);
    let (_v, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);
    mock.set_deposit_rate_bps(&5_997); // odd, non-1:1 share price
    s.client.deploy_to_vault(&(bps_of(2 * MAX_STAKE, MAX_DEPLOY_BPS) - 11), &0);
    mock.set_rate_bps(&16_675); // redemption at the matching odd rate
    s.client.withdraw(&a.0, &a.1);
    assert_invariant(&s);
}

// -----------------------------------------------------------------------
// Push
// -----------------------------------------------------------------------

#[test]
fn no_push_before_the_admins_first_deposit() {
    let env = new_env();
    let s = setup(&env);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    staked_wallet_amount(&env, &s, MAX_STAKE);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
}

#[test]
fn no_push_below_the_threshold() {
    let env = new_env();
    let s = setup(&env);
    for _ in 0..20 {
        staked_wallet_amount(&env, &s, MAX_STAKE);
    }
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&bps_of(20 * MAX_STAKE, MAX_DEPLOY_BPS), &0);
    // A MIN stake adds room of 80% of itself: below 1% of this pool's capacity.
    let small = MIN_STAKE;
    assert!(bps_of(small, MAX_DEPLOY_BPS) < bps_of(s.client.get_capacity() + small, AUTO_PUSH_MIN_BPS));
    let deployed = s.client.get_total_deployed_asset();
    staked_wallet_amount(&env, &s, small);
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
}

#[test]
fn backing_is_pushed_once_it_counts_toward_capacity() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, _m) = pool_at_ceiling(&env, &s);
    let deployed = s.client.get_total_deployed_asset();
    let backer = new_funded_address(&env, &s, 10 * MAX_STAKE);
    s.client.back(&backer, &(10 * MAX_STAKE));
    // Unmatured backing is not capacity, so the line hasn't moved yet.
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    advance_ledgers(
        &env,
        (crate::types::BACKER_MATURITY_SECONDS / SECONDS_PER_LEDGER) as u32 + 1,
    );
    s.client.mature_backing(&backer);
    assert_eq!(
        s.client.get_total_deployed_asset(),
        bps_of(s.client.get_capacity(), MAX_DEPLOY_BPS)
    );
    assert_invariant(&s);
}

#[test]
fn a_failing_vault_never_blocks_a_stake() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, mock) = pool_at_ceiling(&env, &s);
    mock.set_fail_deposit(&true);
    let deployed = s.client.get_total_deployed_asset();
    let (c, _) = staked_wallet_amount(&env, &s, MAX_STAKE);
    assert!(s.client.is_eligible(&c));
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    assert_invariant(&s);
}

#[test]
fn a_bad_push_rate_keeps_the_stake_and_records_the_real_shares() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, mock) = pool_at_ceiling(&env, &s);
    let shares_before = s.client.get_total_deployed_shares();
    let asset_before = s.client.get_total_deployed_asset();
    mock.set_deposit_rate_bps(&8_000); // 20% fewer shares than the reference rate
    staked_wallet_amount(&env, &s, MAX_STAKE);
    let pushed = s.client.get_total_deployed_asset() - asset_before;
    assert!(pushed > 0);
    // Real shares, not the expected ones: accounting stays true (decision A).
    assert_eq!(s.client.get_total_deployed_shares() - shares_before, bps_of(pushed, 8_000));
}

#[test]
fn a_push_never_touches_money_owed_to_a_live_claim() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, _m) = pool_at_ceiling(&env, &s);
    advance_past_time_gate(&env);
    let entitlement = MAX_STAKE / 4; // inside the tier ceiling and the stress cap
    submit_claim_signed(&env, &s, &s.oracle, &a.0, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env));
    assert!(s.client.get_total_allocated() > 0);
    for _ in 0..5 {
        staked_wallet_amount(&env, &s, MAX_STAKE);
        assert!(s.client.get_liquid_balance() >= s.client.get_total_allocated());
    }
}

// -----------------------------------------------------------------------
// Adversarial: each exit path, can it be gamed?
// -----------------------------------------------------------------------

/// Stake / withdraw churn: every cycle forces a push and a pull. The churner
/// gets back exactly what they put in, and nobody else is short.
#[test]
fn churning_stakes_to_force_pulls_and_pushes_gains_nothing() {
    let env = new_env();
    let s = setup(&env);
    let (a, b, _m) = pool_at_ceiling(&env, &s);
    let churner = new_funded_address(&env, &s, MAX_STAKE);
    for i in 0..10 {
        let ben = Address::generate(&env);
        s.client.stake(&churner, &MAX_STAKE, &ben);
        s.client.withdraw(&churner, &ben);
        assert_eq!(balance(&env, &s, &ben), MAX_STAKE, "cycle {}", i);
        let back = balance(&env, &s, &ben);
        TokenClient::new(&env, &s.token_id).transfer(&ben, &churner, &back);
        assert_invariant(&s);
    }
    // The two honest stakers still exit in full.
    s.client.withdraw(&a.0, &a.1);
    s.client.withdraw(&b.0, &b.1);
    assert_eq!(balance(&env, &s, &a.1), MAX_STAKE);
    assert_eq!(balance(&env, &s, &b.1), MAX_STAKE);
}

/// Donating straight to the pool inflates idle cash; the push moves it into
/// the vault. The donor can't get it back and nobody's accounting breaks.
#[test]
fn a_donation_cannot_be_used_to_steer_the_vault() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, _m) = pool_at_ceiling(&env, &s);
    let donor = new_funded_address(&env, &s, 50 * MAX_STAKE);
    TokenClient::new(&env, &s.token_id).transfer(&donor, &s.contract_id, &(50 * MAX_STAKE));
    let (c, cb) = staked_wallet_amount(&env, &s, MAX_STAKE);
    // The ceiling is set by capacity, not by cash on hand: no over-deployment.
    assert!(s.client.get_total_deployed_asset() <= bps_of(s.client.get_capacity(), MAX_DEPLOY_BPS));
    s.client.withdraw(&a.0, &a.1);
    s.client.withdraw(&c, &cb);
    assert_eq!(balance(&env, &s, &cb), MAX_STAKE);
    assert_invariant(&s);
}

/// Everyone runs for the exit while paused: every emergency exit pulls what
/// it needs, and total_staked ends at exactly 0 (never negative).
#[test]
fn a_paused_bank_run_empties_the_pool_cleanly() {
    let env = new_env();
    let s = setup(&env);
    let mut stakers = std::vec::Vec::new();
    for _ in 0..20 {
        stakers.push(staked_wallet_amount(&env, &s, MAX_STAKE));
    }
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&bps_of(20 * MAX_STAKE, MAX_DEPLOY_BPS), &0);
    s.client.pause();
    for (staker, _) in &stakers {
        s.client.emergency_exit(staker);
        assert_invariant(&s);
    }
    assert_eq!(s.client.get_total_staked(), 0);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
}

/// A vault loss during a run: early exits are paid in full (accepted and
/// disclosed, mechanism review 2026-09-24); the last one out gets the typed
/// error only when the money is really gone, and total_staked never goes
/// negative.
#[test]
fn a_run_during_a_vault_loss_is_bounded_and_never_underflows() {
    let env = new_env();
    let s = setup(&env);
    let mut stakers = std::vec::Vec::new();
    for _ in 0..10 {
        stakers.push(staked_wallet_amount(&env, &s, MAX_STAKE));
    }
    let (_v, mock) = with_vault(&env, &s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&bps_of(10 * MAX_STAKE, MAX_DEPLOY_BPS), &0);
    mock.set_rate_bps(&(BPS - 400));
    let mut paid = 0;
    let mut refused = 0;
    for (staker, ben) in &stakers {
        match s.client.try_withdraw(staker, ben) {
            Ok(Ok(())) => paid += 1,
            Err(Ok(PoolError::InsufficientLiquidity)) => refused += 1,
            other => panic!("unexpected: {:?}", other),
        }
        assert!(s.client.get_total_staked() >= 0);
    }
    assert!(paid >= 9, "only the real loss can refuse anyone (paid {paid})");
    assert!(refused <= 1);
}

/// A claim's payout is capped by the daily outflow cap; the pull can't be
/// used to take more than that, even with a full vault behind it.
#[test]
fn a_pull_cannot_lift_a_stream_past_its_daily_cap() {
    let env = new_env();
    let s = setup(&env);
    let (a, b, _m) = pool_at_ceiling(&env, &s);
    advance_past_time_gate(&env);
    let entitlement = MAX_STAKE / 4;
    let id = submit_claim_signed(&env, &s, &s.oracle, &a.0, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env));
    s.client.approve_claim(&id);
    // `b` leaves first, so the stream below has to go through the vault.
    s.client.withdraw(&b.0, &b.1);
    advance_ledgers(&env, crate::types::COOLDOWN_LEDGERS + crate::types::VESTING_LEDGERS);
    let paid = s.client.claim_stream(&id, &a.1);
    assert!(paid < entitlement, "one day's stream is capped, not the whole claim");
    assert_eq!(
        s.client.try_claim_stream(&id, &a.1),
        Err(Ok(PoolError::DailyOutflowCapReached))
    );
}

/// Many stakers, random-order entries and exits: invariant after every step,
/// deployment never above the ceiling after a push, and everyone out in full.
#[test]
fn stress_many_stakers_in_and_out() {
    let env = new_env();
    let s = setup(&env);
    let seed = staked_wallet_amount(&env, &s, MAX_STAKE);
    with_vault(&env, &s, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&bps_of(MAX_STAKE, MAX_DEPLOY_BPS), &0);
    let mut live = std::vec::Vec::new();
    live.push(seed);
    for i in 0..60u32 {
        let amount = MIN_STAKE + (i as i128 * 7_919_111) % (MAX_STAKE - MIN_STAKE);
        let deployed_before = s.client.get_total_deployed_asset();
        live.push(staked_wallet_amount(&env, &s, amount));
        // A push never goes above the line (cash-paid exits can leave the
        // vault above it until the next pull refills: known, harmless drift).
        if s.client.get_total_deployed_asset() > deployed_before {
            assert!(s.client.get_total_deployed_asset() <= bps_of(s.client.get_capacity(), MAX_DEPLOY_BPS));
        }
        if i % 3 == 2 {
            let (st, ben) = live.remove((i as usize * 31) % live.len());
            let owed = s.client.get_withdrawable_amount(&st);
            s.client.withdraw(&st, &ben);
            assert_eq!(balance(&env, &s, &ben), owed);
        }
        assert_invariant(&s);
    }
    for (st, ben) in live {
        let owed = s.client.get_withdrawable_amount(&st);
        s.client.withdraw(&st, &ben);
        assert_eq!(balance(&env, &s, &ben), owed);
    }
    assert_eq!(s.client.get_total_staked(), 0);
}

// -----------------------------------------------------------------------
// Adversarial: can money coming in be stolen? (2026-09-24)
// Backer signatures are covered in backer_tests (`only_auth_is`).
// -----------------------------------------------------------------------

/// Nobody can stake or back with someone else's money: with no signatures
/// at all, both calls fail and the victim's balance doesn't move.
#[test]
fn stake_and_back_need_the_owners_own_signature() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, _m) = pool_at_ceiling(&env, &s);
    let victim = new_funded_address(&env, &s, 2 * MAX_STAKE);
    let thief_ben = Address::generate(&env);
    env.set_auths(&[]);
    assert!(s.client.try_stake(&victim, &MAX_STAKE, &thief_ben).is_err());
    assert!(s.client.try_back(&victim, &MAX_STAKE).is_err());
    env.mock_all_auths();
    assert_eq!(balance(&env, &s, &victim), 2 * MAX_STAKE);
}

/// The push adds no outside signer: a stake asks for the staker's signature
/// and nothing else (the pool authorises its own vault deposit).
#[test]
fn a_stake_that_pushes_asks_only_the_staker_to_sign() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, _m) = pool_at_ceiling(&env, &s);
    let deployed = s.client.get_total_deployed_asset();
    let staker = new_funded_address(&env, &s, MAX_STAKE);
    s.client.stake(&staker, &MAX_STAKE, &Address::generate(&env));
    // Read the recorded signatures before any other call replaces them.
    let auths = env.auths();
    assert!(s.client.get_total_deployed_asset() > deployed, "this stake pushed");
    assert_eq!(auths.len(), 1, "exactly one outside signer");
    assert_eq!(auths[0].0, staker);
}

/// Money pushed into the vault is credited to the pool and nobody else.
#[test]
fn pushed_money_belongs_to_the_pool_alone() {
    let env = new_env();
    let s = setup(&env);
    let (a, _b, mock) = pool_at_ceiling(&env, &s);
    let (staker, _) = staked_wallet_amount(&env, &s, MAX_STAKE);
    assert_eq!(mock.balance(&s.contract_id), s.client.get_total_deployed_shares());
    assert_eq!(mock.balance(&staker), 0);
    assert_eq!(mock.balance(&a.0), 0);
}

/// A fresh stake can't be taken out by anyone else: not without the staker's
/// signature, and not to a beneficiary the staker didn't name.
#[test]
fn a_fresh_stake_cannot_be_taken_out_by_someone_else() {
    let env = new_env();
    let s = setup(&env);
    let (_a, _b, _m) = pool_at_ceiling(&env, &s);
    let (victim, victim_ben) = staked_wallet_amount(&env, &s, MAX_STAKE);
    let thief = Address::generate(&env);
    env.set_auths(&[]);
    assert!(s.client.try_withdraw(&victim, &thief).is_err());
    assert!(s.client.try_withdraw(&victim, &victim_ben).is_err());
    assert!(s.client.try_emergency_exit(&victim).is_err());
    env.mock_all_auths();
    assert_eq!(
        s.client.try_withdraw(&victim, &thief),
        Err(Ok(PoolError::WrongBeneficiary))
    );
    assert_eq!(balance(&env, &s, &thief), 0);
    assert!(s.client.is_eligible(&victim));
}

/// A claim can't be streamed by someone else, even with the vault ready to
/// pull for it.
#[test]
fn a_claim_cannot_be_streamed_by_someone_else() {
    let env = new_env();
    let s = setup(&env);
    let (a, b, _m) = pool_at_ceiling(&env, &s);
    advance_past_time_gate(&env);
    let id = submit_claim_signed(
        &env, &s, &s.oracle, &a.0, &tx_hash(&env, 1), &(MAX_STAKE / 4), &TIER_C, &now_ts(&env),
    );
    s.client.approve_claim(&id);
    s.client.withdraw(&b.0, &b.1);
    advance_ledgers(&env, crate::types::COOLDOWN_LEDGERS + crate::types::VESTING_LEDGERS);
    let thief = Address::generate(&env);
    env.set_auths(&[]);
    assert!(s.client.try_claim_stream(&id, &thief).is_err());
    assert!(s.client.try_claim_stream(&id, &a.1).is_err());
    env.mock_all_auths();
    assert_eq!(s.client.get_claim(&id).unwrap().streamed, 0);
    assert_eq!(balance(&env, &s, &thief), 0);
}
