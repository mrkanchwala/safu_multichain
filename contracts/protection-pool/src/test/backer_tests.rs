#![cfg(test)]
//! v1 (2026-09-22): backers. One test per locked rule ("test
//! everything"): maturity (rule 4), notice + still counted during it (rule 2),
//! only free capital leaves (rule 3), own address + own signature only
//! (rule 1), full amount back, pause behaviour, and every capacity site that
//! now reads `total_staked + total_backed`.

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Env};

use super::common::*;
use super::d2_vault_tests::with_vault;
use crate::error::PoolError;
use crate::settings::SettingKey;
use crate::types::{ClaimStatus, BACKER_MATURITY_SECONDS, SECONDS_PER_DAY};

const TIER_C: u32 = 3;
const BACKING: i128 = 1_000_000_000; // 100 units
const MATURITY_LEDGERS: u32 = (BACKER_MATURITY_SECONDS / SECONDS_PER_LEDGER) as u32;
const DEFAULT_NOTICE_SECONDS: u64 = 30 * SECONDS_PER_DAY;
const NOTICE_LEDGERS: u32 = (DEFAULT_NOTICE_SECONDS / SECONDS_PER_LEDGER) as u32;

fn funded_backer(env: &Env, s: &Setup<'_>, amount: i128) -> Address {
    new_funded_address(env, s, amount)
}

/// Backs `amount` and lets it mature. Returns the backer.
fn matured_backer(env: &Env, s: &Setup<'_>, amount: i128) -> Address {
    let b = funded_backer(env, s, amount);
    s.client.back(&b, &amount);
    advance_ledgers(env, MATURITY_LEDGERS);
    s.client.mature_backing(&b);
    b
}

fn emitted(env: &Env, name: &str) -> bool {
    env.events().all().events().iter().any(|e| match &e.body {
        ContractEventBody::V0(v0) => v0
            .topics
            .first()
            .map_or(false, |t| matches!(t, ScVal::Symbol(sym) if sym.0.as_slice() == name.as_bytes())),
    })
}

fn only_auth_is(env: &Env, who: &Address) {
    let auths = env.auths();
    assert_eq!(auths.len(), 1, "exactly one signer expected");
    assert_eq!(&auths[0].0, who);
}

// -- deposit + maturity (rule 4) -----------------------------------------

#[test]
fn deposit_is_held_but_does_not_count_until_mature() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    assert!(emitted(&env, "backed"));

    assert_eq!(s.client.get_total_backed_pending(), BACKING);
    assert_eq!(s.client.get_total_backed(), 0);
    assert_eq!(s.client.get_capacity(), 0);
    assert_eq!(s.client.get_liquid_balance(), BACKING);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!(r.pending_amount, BACKING);
    assert_eq!(r.pending_matures_at, now_ts(&env) + BACKER_MATURITY_SECONDS);
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

#[test]
fn cannot_mature_one_ledger_early() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS - 1);
    assert_eq!(s.client.try_mature_backing(&b), Err(Ok(PoolError::BackingNotMature)));
    assert_eq!(s.client.get_capacity(), 0);
}

#[test]
fn matures_after_seven_days_and_counts_toward_capacity() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS);
    assert_eq!(s.client.mature_backing(&b), BACKING);
    assert!(emitted(&env, "backing_matured"));

    assert_eq!(s.client.get_total_backed(), BACKING);
    assert_eq!(s.client.get_total_backed_pending(), 0);
    assert_eq!(s.client.get_capacity(), BACKING);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.pending_amount, r.pending_matures_at), (BACKING, 0, 0));
}

#[test]
fn maturing_is_permissionless() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS);
    s.client.mature_backing(&b);
    assert!(env.auths().is_empty(), "no signature needed to mature");
}

#[test]
fn mature_errors() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(s.client.try_mature_backing(&nobody), Err(Ok(PoolError::NoBacker)));
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_mature_backing(&b), Err(Ok(PoolError::NoPendingBacking)));
}

#[test]
fn top_up_restarts_the_clock_for_all_pending_money() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, 2 * BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS - 10);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, 10);
    // The first deposit's original 7 days have passed, but the top-up restarted it.
    assert_eq!(s.client.try_mature_backing(&b), Err(Ok(PoolError::BackingNotMature)));
    advance_ledgers(&env, MATURITY_LEDGERS);
    assert_eq!(s.client.mature_backing(&b), 2 * BACKING);
}

#[test]
fn top_up_after_maturity_banks_the_matured_part_first() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, 2 * BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS);
    s.client.back(&b, &BACKING);
    assert_eq!(s.client.get_total_backed(), BACKING);
    assert_eq!(s.client.get_total_backed_pending(), BACKING);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.pending_amount), (BACKING, BACKING));
}

// -- withdrawal request + notice (rule 2) --------------------------------

#[test]
fn request_starts_the_notice_and_money_still_counts() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let ready_at = s.client.request_backer_withdrawal(&b, &BACKING);
    assert!(emitted(&env, "backer_withdrawal_requested"));
    only_auth_is(&env, &b);

    assert_eq!(ready_at, now_ts(&env) + DEFAULT_NOTICE_SECONDS);
    assert_eq!(s.client.get_capacity(), BACKING, "rule 2: still backs claims during notice");
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.withdraw_amount, r.withdraw_ready_at), (BACKING, ready_at));
}

#[test]
fn notice_follows_the_setting() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let ten_days = 10 * SECONDS_PER_DAY as i128;
    set_setting_via_timelock(&env, &s, SettingKey::BackerNoticeSeconds, ten_days);
    let ready_at = s.client.request_backer_withdrawal(&b, &BACKING);
    assert_eq!(ready_at, now_ts(&env) + ten_days as u64);
}

#[test]
fn only_matured_money_can_be_requested() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    assert_eq!(
        s.client.try_request_backer_withdrawal(&b, &1),
        Err(Ok(PoolError::BackerAmountExceedsBalance))
    );
}

#[test]
fn request_matures_due_money_first() {
    let env = new_env();
    let s = setup(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING);
    advance_ledgers(&env, MATURITY_LEDGERS);
    // Nobody called mature_backing, the request settles it itself.
    s.client.request_backer_withdrawal(&b, &BACKING);
    assert_eq!(s.client.get_total_backed(), BACKING);
}

#[test]
fn request_errors() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(
        s.client.try_request_backer_withdrawal(&nobody, &1),
        Err(Ok(PoolError::NoBacker))
    );
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_request_backer_withdrawal(&b, &0), Err(Ok(PoolError::AmountNotPositive)));
    assert_eq!(
        s.client.try_request_backer_withdrawal(&b, &(BACKING + 1)),
        Err(Ok(PoolError::BackerAmountExceedsBalance))
    );
    s.client.request_backer_withdrawal(&b, &1);
    assert_eq!(
        s.client.try_request_backer_withdrawal(&b, &1),
        Err(Ok(PoolError::BackerWithdrawalPending))
    );
}

#[test]
fn cancel_clears_the_request() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(s.client.try_cancel_backer_withdrawal(&b), Err(Ok(PoolError::NoBackerWithdrawal)));
    s.client.request_backer_withdrawal(&b, &BACKING);
    s.client.cancel_backer_withdrawal(&b);
    assert!(emitted(&env, "backer_withdrawal_cancelled"));
    only_auth_is(&env, &b);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.withdraw_amount, r.withdraw_ready_at), (0, 0));
    s.client.request_backer_withdrawal(&b, &BACKING); // can request again
}

// -- completion: notice, full amount, own address (rule 1) ---------------

#[test]
fn cannot_complete_one_ledger_before_the_notice_ends() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_ledgers(&env, NOTICE_LEDGERS - 1);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::BackerNoticeNotPassed))
    );
}

#[test]
fn completes_in_full_to_the_backer_only() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    let token = soroban_sdk::token::TokenClient::new(&env, &s.token_id);
    assert_eq!(token.balance(&b), 0);

    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_ledgers(&env, NOTICE_LEDGERS);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING);
    assert!(emitted(&env, "backer_withdrawn"));
    only_auth_is(&env, &b);

    assert_eq!(token.balance(&b), BACKING, "full amount back, no deduction");
    assert_eq!(s.client.get_total_backed(), 0);
    assert_eq!(s.client.get_capacity(), 0);
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.withdraw_amount), (0, 0));
}

#[test]
fn partial_withdrawal_leaves_the_rest_counted() {
    let env = new_env();
    let s = setup(&env);
    let b = matured_backer(&env, &s, BACKING);
    s.client.request_backer_withdrawal(&b, &(BACKING / 4));
    advance_ledgers(&env, NOTICE_LEDGERS);
    s.client.complete_backer_withdrawal(&b);
    assert_eq!(s.client.get_capacity(), BACKING - BACKING / 4);
}

#[test]
fn complete_without_request_fails() {
    let env = new_env();
    let s = setup(&env);
    let nobody = Address::generate(&env);
    assert_eq!(s.client.try_complete_backer_withdrawal(&nobody), Err(Ok(PoolError::NoBacker)));
    let b = matured_backer(&env, &s, BACKING);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::NoBackerWithdrawal))
    );
}

// -- rule 3: only free capital leaves -------------------------------------

/// Backer money lets a claim bigger than all staker money be admitted.
/// Returns (backer, claim_id). After activation: staked 0, backed BACKING,
/// allocated = entitlement.
fn claim_backed_by_backer(env: &Env, s: &Setup<'_>, entitlement: i128) -> (Address, soroban_sdk::BytesN<32>) {
    let b = matured_backer(env, s, BACKING);
    let (staker, _) = staked_wallet(env, s);
    advance_past_time_gate(env);
    let id = submit_claim_signed(env, s, &s.oracle, &staker, &tx_hash(env, 1), &entitlement, &TIER_C, &now_ts(env));
    s.client.approve_claim(&id);
    (b, id)
}

#[test]
fn backer_capacity_admits_a_claim_stakers_alone_could_not() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = 4 * MID_STAKE; // > all staker money, <= tier C cap (5x)
    let (_, id) = claim_backed_by_backer(&env, &s, entitlement);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
    assert_eq!(s.client.get_total_allocated(), entitlement);
}

#[test]
fn immature_backing_does_not_admit_claims() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let b = funded_backer(&env, &s, BACKING);
    s.client.back(&b, &BACKING); // not matured
    let id = submit_claim_signed(&env, &s, &s.oracle, &staker, &tx_hash(&env, 1), &(4 * MID_STAKE), &TIER_C, &now_ts(&env));
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Reserved);
}

#[test]
fn capital_needed_by_open_claims_cannot_leave() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = 4 * MID_STAKE;
    let (b, _) = claim_backed_by_backer(&env, &s, entitlement);
    // Free capital is exactly BACKING - entitlement.
    s.client.request_backer_withdrawal(&b, &(BACKING - entitlement + 1));
    advance_ledgers(&env, NOTICE_LEDGERS);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::BackerCapitalNotFree))
    );
    // Nothing changed: request still open, still counted.
    let r = s.client.get_backer(&b).unwrap();
    assert_eq!((r.amount, r.withdraw_amount), (BACKING, BACKING - entitlement + 1));
    assert_eq!(s.client.get_total_backed(), BACKING);
}

#[test]
fn exactly_the_free_capital_can_leave() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = 4 * MID_STAKE;
    let (b, _) = claim_backed_by_backer(&env, &s, entitlement);
    s.client.request_backer_withdrawal(&b, &(BACKING - entitlement));
    advance_ledgers(&env, NOTICE_LEDGERS);
    s.client.complete_backer_withdrawal(&b);
    assert_eq!(s.client.get_capacity(), s.client.get_total_allocated());
}

#[test]
fn blocked_withdrawal_goes_through_once_the_claim_is_released() {
    let env = new_env();
    let s = setup(&env);
    let (b, id) = claim_backed_by_backer(&env, &s, 4 * MID_STAKE);
    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_ledgers(&env, NOTICE_LEDGERS);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::BackerCapitalNotFree))
    );
    s.client.cancel_claim(&id); // releases the allocation
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING);
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
    assert_eq!(
        s.client.try_deploy_to_vault(&(ceiling + 1), &0),
        Err(Ok(PoolError::DeployExceedsCeiling))
    );
    s.client.deploy_to_vault(&ceiling, &0);

    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_ledgers(&env, NOTICE_LEDGERS);
    // v1: only a vault refusal stops it now.
    mock.set_fail_withdraw(&true);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::InsufficientLiquidity))
    );
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
    advance_ledgers(&env, NOTICE_LEDGERS);
    assert_eq!(s.client.complete_backer_withdrawal(&b), BACKING);
}

#[test]
fn rule_3_still_applies_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let (b, _) = claim_backed_by_backer(&env, &s, 4 * MID_STAKE);
    s.client.pause();
    s.client.request_backer_withdrawal(&b, &BACKING);
    advance_ledgers(&env, NOTICE_LEDGERS);
    assert_eq!(
        s.client.try_complete_backer_withdrawal(&b),
        Err(Ok(PoolError::BackerCapitalNotFree))
    );
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
    // With BACKING matured, 25% of capacity covers it.
    let (_, id) = claim_backed_by_backer(&env, &s, 4 * MID_STAKE);
    assert!(bps_of(MID_STAKE, 2_500) < 4 * MID_STAKE);
    assert_eq!(s.client.get_claim(&id).unwrap().status, ClaimStatus::Active);
}

#[test]
fn claim_snapshot_includes_backer_capacity() {
    let env = new_env();
    let s = setup(&env);
    let (_, id) = claim_backed_by_backer(&env, &s, 4 * MID_STAKE);
    // Snapshot is read before the claimant's own forfeiture.
    assert_eq!(s.client.get_claim(&id).unwrap().total_staked_snapshot, MID_STAKE + BACKING);
}

#[test]
fn yield_on_matured_backer_money_goes_to_backers() {
    // r3 (2026-09-29): was `yield_on_backer_money_goes_to_the_protocol`.
    // Backers now earn on matured money (BackerYieldBps default 100%).
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

    // Each side's capital earned half; both defaults are 100%, so the
    // protocol keeps only index rounding dust.
    let (staker_reserved, backer_reserved) = s.client.get_yield_reserved();
    assert!(y / 2 - staker_reserved <= 1 && y / 2 - backer_reserved <= 1);
    assert_eq!(s.client.get_yield_balance(), y - staker_reserved - backer_reserved);
    assert!(s.client.get_yield_balance() <= 2);
    assert_eq!(s.client.get_withdrawable_amount(&staker), MID_STAKE + staker_reserved);
    assert_eq!(s.client.get_backer_yield_owed(&backer), backer_reserved);
}
