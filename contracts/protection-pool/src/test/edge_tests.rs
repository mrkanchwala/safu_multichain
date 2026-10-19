//! Single-rule edge cases, each pinned to the exact boundary or value the
//! rule names.

use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::token::TokenClient;

use super::common::*;
use super::d2_vault_tests::with_vault;
use crate::error::PoolError;
use crate::settings::{SettingKey, SETTINGS_TIMELOCK_SECONDS};
use crate::types::{BPS_DENOMINATOR, STORAGE_VERSION, SUSPEND_MAX_SECONDS};

const TIER_C: u32 = 3;

// -- storage version ---------------------------------------------------------

#[test]
fn migrate_brings_an_older_layout_up_to_this_version_once() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(s.client.get_storage_version(), STORAGE_VERSION);
    env.as_contract(&s.contract_id, || crate::storage::set_storage_version(&env, 0));
    assert_eq!(s.client.get_storage_version(), 0);
    assert_eq!(s.client.migrate(), STORAGE_VERSION);
    assert_eq!(s.client.get_storage_version(), STORAGE_VERSION);
    assert_eq!(s.client.migrate(), STORAGE_VERSION);
}

// -- suspension lapse ----------------------------------------------------------

#[test]
fn the_admin_alone_may_suspend_again_exactly_one_length_after_the_last_ended() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    s.client.suspend_stake(&w);
    // Ends after one length; the admin alone waits one more length.
    env.ledger().with_mut(|li| li.timestamp += 2 * SUSPEND_MAX_SECONDS - 1);
    assert_eq!(s.client.try_suspend_stake(&w), Err(Ok(PoolError::SuspendNeedsSecondRole)));
    env.ledger().with_mut(|li| li.timestamp += 1);
    s.client.suspend_stake(&w);
    assert!(!s.client.is_eligible(&w));
}

// -- points rate change ------------------------------------------------------------

#[test]
fn a_backer_points_rate_change_never_reprices_the_time_before_it() {
    let env = new_env();
    let s = setup(&env);
    let b = new_funded_address(&env, &s, MID_STAKE);
    s.client.back(&b, &MID_STAKE);
    advance_days(&env, 10);
    let key = SettingKey::PointsRateBacker;
    let doubled = 2 * crate::points::POINTS_RATE_ONE;
    s.client.propose_setting(&key, &doubled);
    s.client.approve_setting(&key, &doubled);
    advance_ledgers(&env, (SETTINGS_TIMELOCK_SECONDS / SECONDS_PER_LEDGER) as u32 + 1);
    let before = s.client.get_backer_points(&b);
    assert!(before > 0);
    s.client.execute_setting(&key);
    assert_eq!(s.client.get_backer_points(&b), before, "earned points repriced");
}

// -- exits pay the yield ---------------------------------------------------------------

#[test]
fn a_withdrawal_pays_principal_plus_the_yield_earned() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    let (v, mock) = with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8_000 / 10_000), &0);
    mock.set_rate_bps(&11_000);
    mock.set_deposit_rate_bps(&(10_000 * 10_000 / 11_000));
    s.token_admin.mint(&v, &MID_STAKE);
    let_growth_through(&env, 1_000);
    s.client.harvest();
    let owed = s.client.get_staker_yield_owed(&staker);
    assert!(owed > 0);
    advance_past_time_gate(&env);
    let token = TokenClient::new(&env, &s.token_id);
    let before = token.balance(&ben);
    s.client.withdraw(&staker, &ben);
    assert_eq!(token.balance(&ben) - before, MID_STAKE + owed);
    // Nothing left to collect after the exit.
    assert_eq!(s.client.try_claim_yield(&staker, &ben), Err(Ok(PoolError::NoActiveStake)));
}

// -- claim day share ---------------------------------------------------------------------

#[test]
fn the_day_share_is_worked_from_what_the_claim_is_still_owed() {
    let env = new_env();
    let s = setup(&env);
    for _ in 0..4 {
        staked_wallet(&env, &s);
    }
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let id = submit_claim_signed(&env, &s, &s.oracle, &w, &tx_hash(&env, 1), &MID_STAKE, &TIER_C, &now_ts(&env));
    s.client.approve_claim(&id);
    env.as_contract(&s.contract_id, || {
        let mut claim = crate::storage::get_claim(&env, &id).unwrap();
        claim.streamed = claim.entitlement / 3;
        let remaining = claim.entitlement - claim.streamed;
        let capacity = crate::storage::get_capacity(&env);
        let allocated = 4 * remaining;
        crate::storage::set_payout_snap(&env, &(7, capacity, allocated));
        let base = capacity.max(claim.total_staked_snapshot);
        let cap = base * crate::claim::dynamic_outflow_bps(&env, base) / BPS_DENOMINATOR;
        let want = (cap * remaining + allocated - 1) / allocated;
        assert_eq!(crate::claim::daily_share(&env, &claim, 7), want);
    });
}
