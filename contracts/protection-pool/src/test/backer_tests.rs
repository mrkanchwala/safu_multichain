#![cfg(test)]
//! v1 (2026-09-22): backers. One test per locked rule ("test
//! everything"). The 2026-10-19 rework rewrote the deposit and withdrawal
//! halves: backing counts from day 1 by default with a clock per deposit
//! when maturity is set, and the notice became a daily drip with a
//! fair share of each day's free capital. Unchanged: only free
//! capital leaves (rule 3), own address + own signature only (rule 1), full
//! amount back, pause behaviour, and every capacity site that reads
//! `total_staked + total_backed`.

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Env};

use super::common::*;
use super::d2_vault_tests::with_vault;
use crate::error::PoolError;
use crate::settings::SettingKey;
use crate::types::{ClaimStatus, MAX_PENDING_DEPOSITS, SECONDS_PER_DAY};

const TIER_C: u32 = 3;
const BACKING: i128 = 1_000_000_000; // 100 units
const WEEK_SECS: u64 = 7 * SECONDS_PER_DAY;
const WEEK_LEDGERS: u32 = (WEEK_SECS / SECONDS_PER_LEDGER) as u32;

fn funded_backer(env: &Env, s: &Setup<'_>, amount: i128) -> Address {
    new_funded_address(env, s, amount)
}

/// Backs `amount`. With the default maturity (0) it counts at once.
fn matured_backer(env: &Env, s: &Setup<'_>, amount: i128) -> Address {
    let b = funded_backer(env, s, amount);
    s.client.back(&b, &amount);
    b
}

/// Turns the 7-day maturity on (it is off by default since 2026-10-16).
fn with_week_maturity(env: &Env, s: &Setup<'_>) {
    set_setting_via_timelock(env, s, SettingKey::BackerMaturitySecs, WEEK_SECS as i128);
}

fn emitted(env: &Env, name: &str) -> bool {
    env.events().all().events().iter().any(|e| match &e.body {
        ContractEventBody::V0(v0) => v0
            .topics
            .first()
            .is_some_and(|t| matches!(t, ScVal::Symbol(sym) if sym.0.as_slice() == name.as_bytes())),
    })
}

fn only_auth_is(env: &Env, who: &Address) {
    let auths = env.auths();
    assert_eq!(auths.len(), 1, "exactly one signer expected");
    assert_eq!(&auths[0].0, who);
}

fn free_capital(s: &Setup<'_>) -> i128 {
    s.client.get_capacity() - s.client.get_total_allocated()
}

/// Collects once a day until the request closes. Returns the days taken.
fn drip_to_the_end(env: &Env, s: &Setup<'_>, b: &Address) -> u32 {
    let mut days = 0;
    while s.client.get_backer(b).unwrap().withdraw_amount > 0 {
        advance_days(env, 1);
        s.client.complete_backer_withdrawal(b);
        days += 1;
        assert!(days <= 200, "drip never ended");
    }
    days
}

// -- deposit: counts from day 1 by default ---------------------

#[test]
fn backing_counts_and_earns_at_once_by_default() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    assert!(emitted(&env, "backed"));
    assert_eq!(s.client.get_total_backed(), BACKING);
    assert_eq!(s.client.get_total_backed_pending(), 0);
    assert_eq!(s.client.get_capacity(), BACKING);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.pending.len()), (BACKING, 0));
    assert_eq!(s.client.try_mature_backing(&b), Err(Ok(PoolError::NoPendingBacking)));
}

#[test]
fn back_needs_the_backers_signature() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    only_auth_is(&env, &b);
}

#[test]
fn back_rejects_zero_and_negative() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_back(&b, &0), Err(Ok(PoolError::AmountNotPositive)));
    assert_eq!(s.client.try_back(&b, &-1), Err(Ok(PoolError::AmountNotPositive)));
}

#[test]
fn back_is_blocked_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.pause();
    assert_eq!(s.client.try_back(&b, &BACKING), Err(Ok(PoolError::Paused)));
}

// -- maturity when set: a clock per deposit -------------------------------

#[test]
fn with_maturity_set_a_deposit_waits_then_counts() {
    let env = new_env();
    let s = setup(&env);
    with_week_maturity(&env, &s);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    assert_eq!((s.client.get_total_backed_pending(), s.client.get_capacity()), (BACKING, 0));
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!(r.pending.get(0).unwrap().matures_at, now_ts(&env) + WEEK_SECS);

    advance_ledgers(&env, WEEK_LEDGERS - 1);
    assert_eq!(s.client.try_mature_backing(&b), Err(Ok(PoolError::BackingNotMature)));
    advance_ledgers(&env, 1);
    assert_eq!(s.client.mature_backing(&b), BACKING);
    assert!(emitted(&env, "backing_matured"));
    assert!(env.auths().is_empty(), "maturing is permissionless");
    assert_eq!((s.client.get_total_backed(), s.client.get_capacity()), (BACKING, BACKING));
}

#[test]
fn a_new_deposit_never_restarts_an_older_ones_clock() {
    // Confirmed 2026-10-16: fixes the old "top-up restarts all pending money".
    let env = new_env();
    let s = setup(&env);
    with_week_maturity(&env, &s);
    let b = funded_backer(&env, &s, 2 * BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, WEEK_LEDGERS - 10);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, 10);
    assert_eq!(s.client.mature_backing(&b), BACKING, "first deposit counts on its own day");
    assert_eq!(s.client.get_total_backed_pending(), BACKING);
    advance_ledgers(&env, WEEK_LEDGERS);
    assert_eq!(s.client.mature_backing(&b), BACKING);
}

#[test]
fn at_most_eight_deposits_wait_at_once() {
    let env = new_env();
    let s = setup(&env);
    with_week_maturity(&env, &s);
    let n = MAX_PENDING_DEPOSITS as i128;
    let b = funded_backer(&env, &s, (n + 1) * BACKING);
    for _ in 0..n {
        s.client.back(&b, &BACKING);
    }
    assert_eq!(s.client.try_back(&b, &BACKING), Err(Ok(PoolError::TooManyPendingDeposits)));
    advance_ledgers(&env, WEEK_LEDGERS);
    s.client.back(&b, &BACKING); // the matured ones count first, freeing slots
    assert_eq!(s.client.get_total_backed(), n * BACKING);
}

#[test]
fn mature_errors() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(s.client.try_mature_backing(&nobody), Err(Ok(PoolError::NoBacker)));
}

#[test]
fn only_counted_money_can_be_requested() {
    let env = new_env();
    let s = setup(&env);
    with_week_maturity(&env, &s);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    assert_eq!(
        s.client.try_request_backer_withdrawal(&b, &1),
        Err(Ok(PoolError::BackerAmountExceedsBalance))
    );
    advance_ledgers(&env, WEEK_LEDGERS);
    // Nobody called mature_backing; the request settles it itself.
    s.client.request_backer_withdrawal(&b, &BACKING);
    assert_eq!(s.client.get_total_backed(), BACKING);
}

// -- request + drip (rule 2) --------------------------------------

#[test]
fn request_opens_a_drip_and_money_still_counts() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let ready_at = s.client.request_backer_withdrawal(&b, &BACKING);
    assert!(emitted(&env, "backer_withdrawal_requested"));
    only_auth_is(&env, &b);
    // Idle pool: 7-day drip, the last drip day is 6 days out.
    assert_eq!(ready_at, now_ts(&env) + 6 * SECONDS_PER_DAY);
    assert_eq!(s.client.get_capacity(), BACKING, "rule 2: still backs claims until paid");
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.withdraw_amount, r.withdraw_paid, r.withdraw_started_at), (BACKING, 0, now_ts(&env)));
}

#[test]
fn idle_pool_drips_out_in_seven_daily_parts() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let token = soroban_sdk::token::TokenClient::new(&env, &s.token_id);
    s.client.request_backer_withdrawal(&b, &BACKING);

    assert_eq!(s.client.get_backer_collectable(&b), BACKING / 7);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING / 7);
    assert!(emitted(&env, "backer_withdrawn"));
    only_auth_is(&env, &b);
    assert_eq!(s.client.get_capacity(), BACKING - BACKING / 7, "paid part stops counting");
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::NothingToCollectYet)),
        "one day's part per day"
    );
    assert_eq!(s.client.get_backer_collectable(&b), 0);

    advance_days(&env, 1);
    assert_eq!(s.client.complete_backer_withdrawal(&b), 2 * BACKING / 7 - BACKING / 7);
    assert_eq!(drip_to_the_end(&env, &s, &b), 5);
    assert_eq!(token.balance(&b), BACKING, "full amount back, no deduction");
    assert_eq!((s.client.get_total_backed(), s.client.get_capacity()), (0, 0));
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.withdraw_amount, r.withdraw_paid), (0, 0, 0));
}

#[test]
fn a_missed_day_carries_forward() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_days(&env, 3);
    assert_eq!(s.client.complete_backer_withdrawal(&b), 4 * BACKING / 7);
    advance_days(&env, 10);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING - 4 * BACKING / 7);
    assert_eq!(s.client.get_backer(&b).unwrap().withdraw_amount, 0, "request closed");
}

#[test]
fn drip_length_follows_the_settings() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    set_setting_via_timelock(&env, &s, SettingKey::DripMinSecs, 2 * SECONDS_PER_DAY as i128);
    s.client.request_backer_withdrawal(&b, &BACKING);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING / 2);
}

#[test]
fn request_errors() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(s.client.try_request_backer_withdrawal(&nobody, &1), Err(Ok(PoolError::NoBacker)));
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_request_backer_withdrawal(&b, &0), Err(Ok(PoolError::AmountNotPositive)));
    assert_eq!(
        s.client.try_request_backer_withdrawal(&b, &(BACKING + 1)),
        Err(Ok(PoolError::BackerAmountExceedsBalance))
    );
    s.client.request_backer_withdrawal(&b, &1);
    assert_eq!(s.client.try_request_backer_withdrawal(&b, &1), Err(Ok(PoolError::BackerWithdrawalPending)));
}

#[test]
fn cancel_stops_the_drip_and_the_rest_stays() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_cancel_backer_withdrawal(&b), Err(Ok(PoolError::NoBackerWithdrawal)));
    s.client.request_backer_withdrawal(&b, &BACKING);
    s.client.complete_backer_withdrawal(&b);
    s.client.cancel_backer_withdrawal(&b);
    assert!(emitted(&env, "backer_withdrawal_cancelled"));
    only_auth_is(&env, &b);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.withdraw_amount, r.withdraw_paid), (BACKING - BACKING / 7, 0, 0));
    assert_eq!(s.client.get_capacity(), BACKING - BACKING / 7);
    s.client.request_backer_withdrawal(&b, &r.amount); // can request again
}

#[test]
fn complete_without_request_fails() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(s.client.try_complete_backer_withdrawal(&nobody), Err(Ok(PoolError::NoBacker)));
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_complete_backer_withdrawal(&b), Err(Ok(PoolError::NoBackerWithdrawal)));
    assert_eq!(s.client.get_backer_collectable(&b), 0);
}

// -- rule 3: only free capital leaves -------------------------------------

/// Backer money lets a claim bigger than all staker money be admitted.
/// Returns (backers, claim_id).
fn claim_backed_by(env: &Env, s: &Setup<'_>, backers: u32, entitlement: i128) -> (std::vec::Vec<Address>, soroban_sdk::BytesN<32>) {
    let bs = (0..backers).map(|_| matured_backer(env, s, BACKING)).collect();
    let (staker, _) = staked_wallet(env, s);
    advance_past_time_gate(env);
    let id = submit_claim_signed(env, s, &s.oracle, &staker, &tx_hash(env, 1), &entitlement, &TIER_C, &now_ts(env));
    s.client.approve_claim(&id);
    (bs, id)
}

#[test]
fn backer_capacity_admits_a_claim_stakers_alone_could_not() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = 4 * MID_STAKE; // > all staker money, <= tier C cap (5x)
    let (_, id) = claim_backed_by(&env, &s, 3, entitlement);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
    assert_eq!(s.client.get_total_allocated(), entitlement);
}

#[test]
fn immature_backing_does_not_admit_claims() {
    let env = new_env();
    let s = setup(&env);
    with_week_maturity(&env, &s);
    let (staker, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let b = funded_backer(&env, &s, 3 * BACKING);
    s.client.back(&b, &(3 * BACKING)); // waiting a week
    let id = submit_claim_signed(&env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &(4 * MID_STAKE), &TIER_C, &now_ts(&env));
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Reserved);
}

#[test]
fn drip_length_runs_from_seven_to_ninety_days_by_utilisation() {
    let env = new_env();
    let s = setup(&env);
    env.as_contract(&s.contract_id, || {
        assert_eq!(crate::backer::drip_days(&env, 0), 7, "idle pool");
        assert_eq!(crate::backer::drip_days(&env, 2_500), 49, "halfway: 7 + 83 / 2, rounded up");
        assert_eq!(crate::backer::drip_days(&env, 5_000), 90, ">= 50% utilisation");
        assert_eq!(crate::backer::drip_days(&env, 9_999), 90, "never longer than the max");
    });
}

#[test]
fn a_loaded_pool_drips_slower() {
    let env = new_env();
    let s = setup(&env);
    let (bs, _) = claim_backed_by(&env, &s, 1, 4 * MID_STAKE);
    let util = s.client.get_total_allocated() * 10_000 / s.client.get_capacity();
    let days = env.as_contract(&s.contract_id, || crate::backer::drip_days(&env, util));
    assert!(days > 7, "a claim open on the pool lengthens the drip");
    let ask = 1_000_000;
    s.client.request_backer_withdrawal(&bs[0], &ask);
    assert_eq!(s.client.complete_backer_withdrawal(&bs[0]), ask / days);
}

#[test]
fn only_free_capital_leaves_and_a_full_day_pays_nothing() {
    let env = new_env();
    let s = setup(&env);
    let (bs, _) = claim_backed_by(&env, &s, 1, 4 * MID_STAKE);
    let free = free_capital(&s);
    assert!(free > 0 && free < BACKING, "scenario: some but not all of one backer is free");
    s.client.request_backer_withdrawal(&bs[0], &BACKING);
    advance_days(&env, 90); // fully unlocked; only free capital limits it now
    assert_eq!(s.client.complete_backer_withdrawal(&bs[0]), free);
    assert_eq!(free_capital(&s), 0);
    advance_days(&env, 1);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&bs[0]),
        Err(Ok(PoolError::BackerCapitalNotFree)),
        "open claims need every dollar: the drip waits"
    );
    // Nothing changed: rest of the request still open and counted.
    let r = s.client.get_backer(&bs[0]).unwrap();
    assert_eq!((r.amount, r.withdraw_amount - r.withdraw_paid), (BACKING - free, BACKING - free));
}

#[test]
fn a_blocked_drip_resumes_once_the_claim_is_released() {
    let env = new_env();
    let s = setup(&env);
    let (bs, id) = claim_backed_by(&env, &s, 1, 4 * MID_STAKE);
    s.client.request_backer_withdrawal(&bs[0], &BACKING);
    advance_days(&env, 90);
    s.client.complete_backer_withdrawal(&bs[0]); // takes the free part
    advance_days(&env, 1);
    assert_eq!(s.client.try_complete_backer_withdrawal(&bs[0]), Err(Ok(PoolError::BackerCapitalNotFree)));
    s.client.cancel_claim(&id); // releases the allocation
    advance_days(&env, 1);
    s.client.complete_backer_withdrawal(&bs[0]);
    assert_eq!(s.client.get_backer(&bs[0]).unwrap().withdraw_amount, 0, "paid out in full");
}

#[test]
fn the_days_free_capital_is_shared_pro_rata_not_first_come() {
    let env = new_env();
    let s = setup(&env);
    let (bs, _) = claim_backed_by(&env, &s, 2, 4 * MID_STAKE);
    let free = free_capital(&s);
    assert!(free < 2 * BACKING, "scenario: the two requests together exceed free capital");
    s.client.request_backer_withdrawal(&bs[0], &BACKING);
    s.client.request_backer_withdrawal(&bs[1], &BACKING);
    advance_days(&env, 90);
    let first = s.client.complete_backer_withdrawal(&bs[0]);
    let second = s.client.complete_backer_withdrawal(&bs[1]);
    assert!((first - free / 2).abs() <= 1 && (second - free / 2).abs() <= 1,
        "each gets half the day's free capital: {first} / {second} of {free}");
}

#[test]
fn rule_3_still_applies_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let (bs, _) = claim_backed_by(&env, &s, 1, 4 * MID_STAKE);
    let free = free_capital(&s);
    s.client.pause();
    s.client.request_backer_withdrawal(&bs[0], &BACKING);
    advance_days(&env, 90);
    assert_eq!(s.client.complete_backer_withdrawal(&bs[0]), free);
}

// -- liquidity ------------------------------------------------------------

#[test]
fn deployed_backer_money_is_pulled_back_on_withdrawal() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let (_v, mock) = with_vault(&env, &s, 8_000);
    // Deploy ceiling includes backer capital.
    let ceiling = BACKING * 8_000 / 10_000;
    assert_eq!(s.client.try_deploy_to_vault(&(ceiling + 1), &0), Err(Ok(PoolError::DeployExceedsCeiling)));
    s.client.deploy_to_vault(&ceiling, &0);

    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_days(&env, 7);
    // v1: only a vault refusal stops it now.
    mock.set_fail_withdraw(&true);
    assert_eq!(s.client.try_complete_backer_withdrawal(&b), Err(Ok(PoolError::InsufficientLiquidity)));
    assert_eq!(s.client.get_total_backed(), BACKING);
    // Vault back: the withdrawal pulls the deployed money itself.
    mock.set_fail_withdraw(&false);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
}

// -- pause: withdrawal path stays open ------------------------------------

#[test]
fn withdrawal_path_works_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    s.client.pause();
    s.client.request_backer_withdrawal(&b, &BACKING);
    s.client.cancel_backer_withdrawal(&b);
    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_days(&env, 7);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING);
}

// -- staker-only limits and yield ------------------------------------------

#[test]
fn backer_money_does_not_use_up_the_pool_cap() {
    let env = new_env();
    let s = setup(&env);
    matured_backer(&env, &s, POOL_CAP);
    staked_wallet(&env, &s); // still admitted
    assert_eq!(s.client.get_total_staked(), MID_STAKE);
}

#[test]
fn stress_cap_is_sized_against_capacity() {
    let env = new_env();
    let s = setup(&env);
    // Staker money alone: 25% of MID_STAKE/day admits far less than 4x stake.
    let (_, id) = claim_backed_by(&env, &s, 3, 4 * MID_STAKE);
    assert!(bps_of(MID_STAKE, 2_500) < 4 * MID_STAKE);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
}

#[test]
fn claim_snapshot_includes_backer_capacity() {
    let env = new_env();
    let s = setup(&env);
    let (_, id) = claim_backed_by(&env, &s, 3, 4 * MID_STAKE);
    // Snapshot is read before the claimant's own forfeiture.
    assert_eq!(s.client.get_claim(&id).unwrap().total_staked_snapshot, MID_STAKE + 3 * BACKING);
}

#[test]
fn yield_on_counted_backer_money_goes_to_backers() {
    // r3 (2026-09-29): backers earn on counted money (BackerYieldBps default 100%).
    let env = new_env();
    let s = setup(&env);
    let (staker, _) = staked_wallet(&env, &s);
    let backer = matured_backer(&env, &s, MID_STAKE); // capacity split 50/50
    let (vault_id, mock) = with_vault(&env, &s, 8_000);
    let deployed = (2 * MID_STAKE) * 8_000 / 10_000;
    s.client.deploy_to_vault(&deployed, &0);
    mock.set_rate_bps(&11_000); // +10%
    let_growth_through(&env, 5_000); // CSO M2: harvest growth limit needs time
    s.token_admin.mint(&vault_id, &deployed); // real tokens to pay above par
    let y = s.client.harvest();
    assert!(y > 0 && deployed / 10 - y <= 2);

    let (staker_reserved, backer_reserved) = s.client.get_yield_reserved();
    assert!(y / 2 - staker_reserved <= 1 && y / 2 - backer_reserved <= 1);
    assert_eq!(s.client.get_yield_balance(), y - staker_reserved - backer_reserved);
    assert!(s.client.get_yield_balance() <= 2);
    assert_eq!(s.client.get_withdrawable_amount(&staker), MID_STAKE + staker_reserved);
    assert_eq!(s.client.get_backer_yield_owed(&backer), backer_reserved);
}
