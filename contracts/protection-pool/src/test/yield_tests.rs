//! r3 (2026-09-29): yield for stakers and backers, taken any time.
//!
//! One test per rule from the r3 eng review
//! (`2026-09-29_plan-eng-review-safu-backer-yield-instant-setup`):
//! pending backer money earns nothing, a staker's yield claim keeps the
//! stake, the additive index pays late joiners exactly, set-aside yield is
//! never deployed or spent on anything else, in-path redemptions credit
//! their growth, a harvest failure never blocks a payout, instant setup is
//! sticky, and forfeited / emergency-exit yield goes where the rules say.
//!
//! Every test ends on `assert_full_invariant`: the pool controls at least
//! what it owes stakers, backers (counted and pending) and the set-aside.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{Address, Env};

use super::common::*;
use super::d2_vault_tests::{with_vault, MockVaultClient};
use crate::error::PoolError;
use crate::settings::SettingKey;
use crate::types::{BACKER_MATURITY_SECONDS, YIELD_INDEX_PRECISION};
use crate::{GovChange, GovKind};

const MATURITY_LEDGERS: u32 = (BACKER_MATURITY_SECONDS / SECONDS_PER_LEDGER) as u32;
const DEFAULT_NOTICE_LEDGERS: u32 = (30 * 86_400 / SECONDS_PER_LEDGER) as u32;
const ENTITLEMENT: i128 = STROOPS_PER_UNIT;
const TIER_C: u32 = 3;
/// Deploy ceiling used by every test here, bps of capacity.
const DEPLOY_BPS: i128 = 8_000;
/// One vault gain step, bps of the redemption rate.
const GAIN_BPS: i128 = 1_000;

fn balance(env: &Env, s: &Setup<'_>, who: &Address) -> i128 {
    TokenClient::new(env, &s.token_id).balance(who)
}

/// liquid + deployed ≥ staked + backed + pending + set-aside yield.
fn assert_full_invariant(s: &Setup<'_>) {
    let held = s.client.get_liquid_balance() + s.client.get_total_deployed_asset();
    let (staker_res, backer_res) = s.client.get_yield_reserved();
    let owed = s.client.get_total_staked()
        + s.client.get_total_backed()
        + s.client.get_total_backed_pending()
        + staker_res
        + backer_res;
    assert!(held >= owed, "INVARIANT: held {} < owed {}", held, owed);
    // Set-aside yield is cash, never deployed.
    assert!(s.client.get_liquid_balance() >= staker_res + backer_res);
}

fn matured_backer(env: &Env, s: &Setup<'_>, amount: i128) -> Address {
    let b = new_funded_address(env, s, amount);
    s.client.back(&b, &amount);
    advance_ledgers(env, MATURITY_LEDGERS);
    s.client.mature_backing(&b);
    b
}

/// Deploys `DEPLOY_BPS` of capacity and returns the mock.
fn deployed<'a>(env: &'a Env, s: &Setup<'a>) -> (Address, MockVaultClient<'a>, i128) {
    let (vault_id, mock) = with_vault(env, s, DEPLOY_BPS);
    let amount = s.client.get_capacity() * DEPLOY_BPS / 10_000;
    s.client.deploy_to_vault(&amount, &0);
    (vault_id, mock, amount)
}

/// Raises the redemption rate to `rate_bps` and funds the vault to pay it.
/// New deposits then mint at that same price, as a real vault does (the
/// mock's 1:1 default would hand every later deposit instant fake growth).
fn grow(s: &Setup<'_>, vault_id: &Address, mock: &MockVaultClient<'_>, rate_bps: i128) {
    mock.set_rate_bps(&rate_bps);
    mock.set_deposit_rate_bps(&(10_000 * 10_000 / rate_bps));
    s.token_admin.mint(vault_id, &s.client.get_capacity());
}

// -----------------------------------------------------------------------
// Rule 1: backers earn on matured money only
// -----------------------------------------------------------------------

#[test]
fn pending_backer_money_earns_nothing_matured_money_earns() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let b = new_funded_address(&env, &s, MID_STAKE);
    s.client.back(&b, &MID_STAKE); // still in its 7-day wait
    let (vault_id, mock, _) = deployed(&env, &s);

    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    assert!(s.client.harvest() > 0);
    assert_eq!(s.client.get_yield_reserved().1, 0, "pending money earned");
    assert_eq!(s.client.get_backer_yield_owed(&b), 0);
    assert_eq!(s.client.try_claim_backer_yield(&b), Err(Ok(PoolError::NoYieldOwed)));

    advance_ledgers(&env, MATURITY_LEDGERS);
    s.client.mature_backing(&b);
    grow(&s, &vault_id, &mock, 10_000 + 2 * GAIN_BPS);
    assert!(s.client.harvest() > 0);
    let owed = s.client.get_backer_yield_owed(&b);
    assert!(owed > 0, "matured money must earn");

    let paid = s.client.claim_backer_yield(&b);
    assert_eq!(paid, owed);
    assert_eq!(balance(&env, &s, &b), owed);
    assert_eq!(s.client.get_total_backed(), MID_STAKE, "principal untouched");
    assert_full_invariant(&s);
}

#[test]
fn backer_yield_setting_is_timelocked_and_only_moves_later_harvests() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let b = matured_backer(&env, &s, MID_STAKE);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let owed_before = s.client.get_backer_yield_owed(&b);
    assert!(owed_before > 0);

    set_setting_via_timelock(&env, &s, SettingKey::BackerYieldBps, 0);
    let protocol_before = s.client.get_yield_balance();
    grow(&s, &vault_id, &mock, 10_000 + 2 * GAIN_BPS);
    assert!(s.client.harvest() > 0);
    assert_eq!(s.client.get_backer_yield_owed(&b), owed_before, "earlier credit kept");
    assert!(s.client.get_yield_balance() > protocol_before, "backer part went to the protocol");
    assert_full_invariant(&s);
}

#[test]
fn completed_backer_withdrawal_pays_unpaid_yield_with_the_principal() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let b = matured_backer(&env, &s, MID_STAKE);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let owed = s.client.get_backer_yield_owed(&b);
    assert!(owed > 0);

    s.client.request_backer_withdrawal(&b, &MID_STAKE);
    advance_ledgers(&env, DEFAULT_NOTICE_LEDGERS);
    s.client.complete_backer_withdrawal(&b);
    assert!(balance(&env, &s, &b) >= MID_STAKE + owed);
    assert_eq!(s.client.get_backer_yield_owed(&b), 0);
    assert_full_invariant(&s);
}

// -----------------------------------------------------------------------
// Rule 2: stakers take yield any time and keep the stake
// -----------------------------------------------------------------------

#[test]
fn staker_claim_yield_keeps_the_stake_and_withdraw_pays_principal_only() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    // No harvest call: the claim harvests by itself (rule 3).
    let before = s.client.get_stake(&staker).unwrap();

    let paid = s.client.claim_yield(&staker, &ben);
    assert!(paid > 0);
    assert_eq!(balance(&env, &s, &ben), paid);
    let after = s.client.get_stake(&staker).unwrap();
    assert_eq!(after.amount, before.amount);
    assert_eq!(after.staked_at_ledger, before.staked_at_ledger, "90-day clock untouched");
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
    assert_eq!(s.client.get_withdrawable_amount(&staker), MID_STAKE);
    assert_eq!(s.client.try_claim_yield(&staker, &ben), Err(Ok(PoolError::NoYieldOwed)));

    // Only the beneficiary can receive it.
    let stranger = Address::generate(&env);
    grow(&s, &vault_id, &mock, 10_000 + 2 * GAIN_BPS);
    assert_eq!(s.client.try_claim_yield(&staker, &stranger), Err(Ok(PoolError::WrongBeneficiary)));

    advance_past_time_gate(&env);
    let ben_before = balance(&env, &s, &ben);
    let owed_now = s.client.get_withdrawable_amount(&staker) - MID_STAKE;
    s.client.withdraw(&staker, &ben);
    let received = balance(&env, &s, &ben) - ben_before;
    assert!(received >= MID_STAKE + owed_now, "principal plus any later yield");
    assert_full_invariant(&s);
}

#[test]
fn late_staker_gets_an_exact_pro_rata_share() {
    // F1: the additive index. Under the old ratio formula B would get
    // c2/2 × PREC / (PREC + x): short by roughly A's first-credit rate.
    let env = new_env();
    let s = setup(&env);
    let (a, _) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let c1 = s.client.get_yield_reserved().0;
    assert!(c1 > 0);
    assert!(s.client.get_yield_index() > YIELD_INDEX_PRECISION);

    let (b, _) = staked_wallet(&env, &s); // same amount, joins after c1
    let reserved_before = s.client.get_yield_reserved().0;
    grow(&s, &vault_id, &mock, 10_000 + 3 * GAIN_BPS);
    assert!(s.client.harvest() > 0);
    let c2 = s.client.get_yield_reserved().0 - reserved_before;
    assert!(c2 > 0);

    let owed_a = s.client.get_staker_yield_owed(&a);
    let owed_b = s.client.get_staker_yield_owed(&b);
    assert!((owed_b - c2 / 2).abs() <= 1, "B {} vs half of c2 {}", owed_b, c2 / 2);
    assert!((owed_a - (c1 + c2 / 2)).abs() <= 2, "A {} vs {}", owed_a, c1 + c2 / 2);
    assert!(owed_a + owed_b <= s.client.get_yield_reserved().0, "never owes more than set aside");
    assert_full_invariant(&s);
}

// -----------------------------------------------------------------------
// Rule 3 + ring-fence: harvest in the payout, set-aside never spent
// -----------------------------------------------------------------------

#[test]
fn a_failed_harvest_never_blocks_paying_what_is_already_credited() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let owed = s.client.get_staker_yield_owed(&staker);
    assert!(owed > 0);

    // More growth, but the vault now refuses every withdrawal.
    grow(&s, &vault_id, &mock, 10_000 + 2 * GAIN_BPS);
    mock.set_fail_withdraw(&true);
    assert_eq!(s.client.harvest(), 0);
    assert_eq!(s.client.claim_yield(&staker, &ben), owed);
    assert_full_invariant(&s);
}

#[test]
fn set_aside_yield_is_never_pushed_into_the_vault() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    assert!(s.client.get_yield_reserved().0 > 0);

    // New stakes trigger the automatic push; the set-aside must stay cash.
    for _ in 0..3 {
        staked_wallet(&env, &s);
        assert_full_invariant(&s);
    }
    let _ = s.client.try_auto_deploy_liquidity();
    assert_full_invariant(&s);
}

#[test]
fn a_keeper_redemption_credits_its_growth_instead_of_redepositing_it() {
    // B1: before r3 this growth became unowned cash and was pushed back
    // into the vault as principal.
    let env = new_env();
    let s = setup(&env);
    let (staker, _) = staked_wallet(&env, &s);
    let (vault_id, mock, amount) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);

    let shares = s.client.get_total_deployed_shares();
    s.client.provide_liquidity(&shares, &0);
    let growth = amount * GAIN_BPS / 10_000;
    assert_eq!(s.client.get_total_extracted_yield(), growth);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
    let (staker_res, _) = s.client.get_yield_reserved();
    assert!(growth - staker_res <= 1, "default 100% to stakers");
    assert_eq!(s.client.get_staker_yield_owed(&staker), staker_res);
    assert_full_invariant(&s);
}

#[test]
fn a_vault_loss_credits_nothing_and_leaves_the_set_aside_alone() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let reserved = s.client.get_yield_reserved();
    let extracted = s.client.get_total_extracted_yield();

    mock.set_rate_bps(&(10_000 - GAIN_BPS));
    assert_eq!(s.client.harvest(), 0);
    assert_eq!(s.client.get_yield_reserved(), reserved);
    assert_eq!(s.client.get_total_extracted_yield(), extracted);
}

// -----------------------------------------------------------------------
// Rule 5: instant setup before the first money, sticky afterwards
// -----------------------------------------------------------------------

fn try_instant_treasury(env: &Env, s: &Setup<'_>) -> Result<(), PoolError> {
    let change = GovChange::Treasury(Address::generate(env));
    s.client.propose_change(&s.admin, &change);
    s.client.approve_change(&s.client.get_co_signer(), &change);
    match s.client.try_execute_change(&GovKind::Treasury) {
        Ok(Ok(())) => Ok(()),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn setup_changes_apply_at_once_before_any_money() {
    let env = new_env();
    let s = setup(&env);
    assert!(!s.client.is_ever_funded());
    assert_eq!(try_instant_treasury(&env, &s), Ok(()));
    // Still two roles: one alone cannot.
    let change = GovChange::Vault(Address::generate(&env));
    s.client.propose_change(&s.admin, &change);
    assert_eq!(s.client.try_execute_change(&GovKind::Vault), Err(Ok(PoolError::GovNotReady)));
}

#[test]
fn the_first_stake_ends_instant_setup() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    assert!(s.client.is_ever_funded());
    assert_eq!(try_instant_treasury(&env, &s), Err(PoolError::GovNotReady));
}

#[test]
fn the_first_backing_ends_instant_setup() {
    let env = new_env();
    let s = setup(&env);
    let b = new_funded_address(&env, &s, MID_STAKE);
    s.client.back(&b, &MID_STAKE);
    assert!(s.client.is_ever_funded());
    assert_eq!(try_instant_treasury(&env, &s), Err(PoolError::GovNotReady));
}

#[test]
fn instant_setup_stays_closed_after_the_pool_empties() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    s.client.withdraw(&staker, &ben);
    assert_eq!(s.client.get_total_staked(), 0);
    assert!(s.client.is_ever_funded(), "never cleared");
    assert_eq!(try_instant_treasury(&env, &s), Err(PoolError::GovNotReady));
}

// -----------------------------------------------------------------------
// Forfeiture and emergency exit
// -----------------------------------------------------------------------

#[test]
fn a_forfeited_stakes_unpaid_yield_goes_to_the_protocol_share() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let (_other, _) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let owed = s.client.get_staker_yield_owed(&staker);
    assert!(owed > 0);
    let (reserved_before, _) = s.client.get_yield_reserved();
    let protocol_before = s.client.get_yield_balance();

    advance_past_time_gate(&env);
    let id = submit_claim_signed(
        &env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &ENTITLEMENT, &TIER_C, &now_ts(&env),
    );
    // While a claim is open the yield cannot be taken out from under it.
    assert!(s.client.try_claim_yield(&staker, &ben).is_err());

    s.client.approve_claim(&id);
    assert_eq!(s.client.get_yield_reserved().0, reserved_before - owed);
    assert_eq!(s.client.get_yield_balance(), protocol_before + owed);
    assert_eq!(s.client.get_staker_yield_owed(&staker), 0);
    assert_full_invariant(&s);
}

#[test]
fn an_emergency_exit_pays_the_unpaid_yield_too() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let owed = s.client.get_staker_yield_owed(&staker);
    assert!(owed > 0);

    s.client.pause();
    s.client.emergency_exit(&staker);
    assert_eq!(balance(&env, &s, &staker), MID_STAKE + owed);
    assert_eq!(s.client.get_yield_reserved().0, 0);
    assert_full_invariant(&s);
}

// -----------------------------------------------------------------------
// Code review 2026-09-29: regressions for two r3 findings
// -----------------------------------------------------------------------

/// A setup change approved while the pool was empty must not stay instantly
/// executable once money arrives: the 7-day wait then applies to it too.
#[test]
fn an_instant_approval_left_unexecuted_waits_once_money_arrives() {
    let env = new_env();
    let s = setup(&env);
    let change = GovChange::Treasury(Address::generate(&env));
    s.client.propose_change(&s.admin, &change);
    s.client.approve_change(&s.client.get_co_signer(), &change);
    staked_wallet(&env, &s); // money arrives before anyone executes
    assert_eq!(s.client.try_execute_change(&GovKind::Treasury), Err(Ok(PoolError::GovNotReady)));
}

/// Growth earned before a staker joins belongs to the stakers who were in:
/// joining (or backing money maturing) recognises pending growth first.
#[test]
fn a_new_staker_does_not_share_growth_earned_before_joining() {
    let env = new_env();
    let s = setup(&env);
    let (a, _) = staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS); // growth not yet harvested
    let (b, _) = staked_wallet(&env, &s);
    s.client.harvest();
    assert_eq!(s.client.get_staker_yield_owed(&b), 0, "B joined after the growth");
    assert!(s.client.get_staker_yield_owed(&a) > 0);
    assert_full_invariant(&s);
}

#[test]
fn newly_matured_backing_does_not_share_growth_earned_before_it_counted() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let b = new_funded_address(&env, &s, MID_STAKE);
    s.client.back(&b, &MID_STAKE);
    let (vault_id, mock, _) = deployed(&env, &s);
    advance_ledgers(&env, MATURITY_LEDGERS);
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS); // earned while b was pending
    s.client.mature_backing(&b);
    s.client.harvest();
    assert_eq!(s.client.get_backer_yield_owed(&b), 0, "pending money earns nothing");
    assert_full_invariant(&s);
}

/// Code review W2 (design decision 2026-09-29): claims may spend the
/// protocol's share of pool cash, and its counter does not go down, so the
/// treasury may take only the surplus above everything the pool owes.
#[test]
fn treasury_takes_only_what_is_left_after_everyone_is_covered() {
    let env = new_env();
    let s = setup(&env);
    set_setting_via_timelock(&env, &s, SettingKey::StakerYieldBps, 5_000);
    let (staker, _) = staked_wallet(&env, &s);
    staked_wallet(&env, &s);
    matured_backer(&env, &s, 20 * MID_STAKE); // room for a large claim
    let (vault_id, mock, _) = deployed(&env, &s);
    gov_apply(&env, &s, GovChange::Treasury(Address::generate(&env)));
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let protocol = s.client.get_yield_balance();
    assert!(protocol > 0);

    advance_past_time_gate(&env);
    let entitlement = 5 * MID_STAKE; // Tier C ceiling: more than the forfeited stake
    let id = submit_claim_signed(
        &env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &entitlement, &TIER_C, &now_ts(&env),
    );
    s.client.approve_claim(&id);
    // The counter never goes down for a claim (it even gains the forfeited
    // stake's unpaid yield, per the forfeiture rule).
    let counter = s.client.get_yield_balance();
    assert!(counter >= protocol);

    // The open claim now owes more than the protocol's cash: nothing left over.
    assert_eq!(s.client.try_withdraw_yield(&counter), Err(Ok(PoolError::ExceedsYieldBalance)));
    assert_eq!(s.client.try_withdraw_yield(&1), Err(Ok(PoolError::ExceedsYieldBalance)));
    assert_full_invariant(&s);
}

#[test]
fn treasury_still_takes_its_full_share_when_nothing_else_is_owed_from_it() {
    let env = new_env();
    let s = setup(&env);
    set_setting_via_timelock(&env, &s, SettingKey::StakerYieldBps, 5_000);
    staked_wallet(&env, &s);
    let (vault_id, mock, _) = deployed(&env, &s);
    let treasury = Address::generate(&env);
    gov_apply(&env, &s, GovChange::Treasury(treasury.clone()));
    grow(&s, &vault_id, &mock, 10_000 + GAIN_BPS);
    s.client.harvest();
    let protocol = s.client.get_yield_balance();
    assert!(protocol > 0);
    s.client.withdraw_yield(&protocol);
    assert_eq!(balance(&env, &s, &treasury), protocol);
    assert_full_invariant(&s);
}
