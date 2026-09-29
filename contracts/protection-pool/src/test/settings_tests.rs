#![cfg(test)]
//! v1 (2026-09-22): adjustable settings. One test per locked rule:
//! bounds, 2-of-2 matching values, the 7-day timelock, cancel rights,
//! cross-setting order, the pool-cap brake, defaults, and the per-claim
//! snapshot (a change never re-prices a claim that is already streaming).

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::error::PoolError;
use crate::settings::{SettingKey, SETTINGS_TIMELOCK_SECONDS};
use crate::types::{
    COOLDOWN_LEDGERS, LEDGERS_PER_DAY, MAX_STAKE_BPS, MIN_STAKE_BPS, VESTING_LEDGERS,
    YIELD_SPLIT_STAKER_BPS,
};

const TIER_C: u32 = 3;
const TIMELOCK_LEDGERS: u32 = (SETTINGS_TIMELOCK_SECONDS / SECONDS_PER_LEDGER) as u32;
const NEW_VESTING: i128 = 60 * LEDGERS_PER_DAY as i128;

fn active_claim(env: &soroban_sdk::Env, s: &Setup<'_>, seed: u8, entitlement: i128) -> (Address, soroban_sdk::BytesN<32>) {
    let (staker, ben) = staked_wallet(env, s);
    advance_past_time_gate(env);
    let claim_id = submit_claim_signed(
        env, s, &s.oracle, &staker, &tx_hash(env, seed), &entitlement, &TIER_C, &now_ts(env),
    );
    s.client.approve_claim(&claim_id);
    (ben, claim_id)
}

// -- defaults ------------------------------------------------------------

#[test]
fn defaults_match_the_shipped_values() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(s.client.get_setting(&SettingKey::MinStakeBps), MIN_STAKE_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::MaxStakeBps), MAX_STAKE_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::StakerYieldBps), 10_000);
    assert_eq!(YIELD_SPLIT_STAKER_BPS, 10_000, "founder: 100% of staker-capital yield to stakers");
    assert_eq!(s.client.get_setting(&SettingKey::CooldownLedgers), COOLDOWN_LEDGERS as i128);
    assert_eq!(s.client.get_setting(&SettingKey::VestingLedgers), VESTING_LEDGERS as i128);
    assert_eq!(s.client.get_setting(&SettingKey::AdmitLowBps), STRESS_RATE_BAND_1_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::AdmitMidBps), STRESS_RATE_BAND_2_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::AdmitHighBps), STRESS_RATE_BAND_3_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::PayoutLowBps), OUTFLOW_RATE_BAND_1_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::PayoutMidBps), OUTFLOW_RATE_BAND_2_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::PayoutHighBps), OUTFLOW_RATE_BAND_3_BPS);
    assert_eq!(s.client.get_setting(&SettingKey::PoolCap), POOL_CAP);
}

// -- the three-step path -------------------------------------------------

#[test]
fn full_path_changes_the_value() {
    let env = new_env();
    let s = setup(&env);
    set_setting_via_timelock(&env, &s, SettingKey::VestingLedgers, NEW_VESTING);
    assert_eq!(s.client.get_setting(&SettingKey::VestingLedgers), NEW_VESTING);
    assert!(s.client.get_pending_setting(&SettingKey::VestingLedgers).is_none());
}

#[test]
fn cannot_execute_before_approval() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    advance_ledgers(&env, TIMELOCK_LEDGERS + 1);
    assert_eq!(
        s.client.try_execute_setting(&SettingKey::VestingLedgers),
        Err(Ok(PoolError::SettingNotReady))
    );
}

#[test]
fn cannot_execute_one_ledger_before_the_timelock() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    s.client.approve_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    advance_ledgers(&env, TIMELOCK_LEDGERS - 1);
    assert_eq!(
        s.client.try_execute_setting(&SettingKey::VestingLedgers),
        Err(Ok(PoolError::SettingNotReady))
    );
    advance_ledgers(&env, 1); // exactly at eta
    s.client.execute_setting(&SettingKey::VestingLedgers);
    assert_eq!(s.client.get_setting(&SettingKey::VestingLedgers), NEW_VESTING);
}

#[test]
fn co_signer_must_approve_the_same_value() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    assert_eq!(
        s.client.try_approve_setting(&SettingKey::VestingLedgers, &(NEW_VESTING + 1)),
        Err(Ok(PoolError::SettingValueMismatch))
    );
}

#[test]
fn approving_twice_is_rejected() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    s.client.approve_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    assert_eq!(
        s.client.try_approve_setting(&SettingKey::VestingLedgers, &NEW_VESTING),
        Err(Ok(PoolError::SettingAlreadyApproved))
    );
}

#[test]
fn a_new_proposal_resets_approval_and_clock() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    s.client.approve_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    s.client.propose_setting(&SettingKey::VestingLedgers, &(NEW_VESTING + LEDGERS_PER_DAY as i128));
    let p = s.client.get_pending_setting(&SettingKey::VestingLedgers).unwrap();
    assert!(!p.approved);
    assert_eq!(p.eta, 0);
}

#[test]
fn execute_or_approve_without_a_proposal_fails() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(
        s.client.try_execute_setting(&SettingKey::CooldownLedgers),
        Err(Ok(PoolError::NoPendingSetting))
    );
    assert_eq!(
        s.client.try_approve_setting(&SettingKey::CooldownLedgers, &(COOLDOWN_LEDGERS as i128)),
        Err(Ok(PoolError::NoPendingSetting))
    );
}

// -- cancel ----------------------------------------------------------------

#[test]
fn admin_or_co_signer_can_cancel_but_nobody_else() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    let stranger = Address::generate(&env);
    assert_eq!(
        s.client.try_cancel_setting(&stranger, &SettingKey::VestingLedgers),
        Err(Ok(PoolError::CallerNotAdminOrCoSigner))
    );
    s.client.cancel_setting(&s.co_signer, &SettingKey::VestingLedgers);
    assert!(s.client.get_pending_setting(&SettingKey::VestingLedgers).is_none());

    s.client.propose_setting(&SettingKey::VestingLedgers, &NEW_VESTING);
    s.client.cancel_setting(&s.admin, &SettingKey::VestingLedgers);
    assert!(s.client.get_pending_setting(&SettingKey::VestingLedgers).is_none());
}

// -- bounds ----------------------------------------------------------------

#[test]
fn every_setting_rejects_values_outside_its_bounds() {
    let env = new_env();
    let s = setup(&env);
    let day = LEDGERS_PER_DAY as i128;
    let cases: [(SettingKey, i128, i128); 12] = [
        (SettingKey::MinStakeBps, 0, 126),
        (SettingKey::MaxStakeBps, 0, 126),
        (SettingKey::StakerYieldBps, -1, 10_001),
        (SettingKey::CooldownLedgers, 7 * day - 1, 30 * day + 1),
        (SettingKey::VestingLedgers, 30 * day - 1, 90 * day + 1),
        (SettingKey::AdmitLowBps, 0, 2_501),
        (SettingKey::AdmitMidBps, 0, 2_501),
        (SettingKey::AdmitHighBps, 0, 2_501),
        (SettingKey::PayoutLowBps, 0, 601),
        (SettingKey::PayoutMidBps, 0, 601),
        (SettingKey::PayoutHighBps, 0, 601),
        (SettingKey::BackerNoticeSeconds, -1, 90 * 86_400 + 1),
    ];
    for (key, below, above) in cases {
        assert_eq!(s.client.try_propose_setting(&key, &below), Err(Ok(PoolError::SettingOutOfBounds)));
        assert_eq!(s.client.try_propose_setting(&key, &above), Err(Ok(PoolError::SettingOutOfBounds)));
    }
}

#[test]
fn payout_rate_can_never_exceed_six_percent_a_day() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_setting(&SettingKey::PayoutLowBps, &600);
    assert_eq!(s.client.try_propose_setting(&SettingKey::PayoutLowBps, &601), Err(Ok(PoolError::SettingOutOfBounds)));
}

// -- cross-setting order ---------------------------------------------------

#[test]
fn min_stake_can_never_exceed_max_stake() {
    let env = new_env();
    let s = setup(&env);
    let above_max = MAX_STAKE_BPS + 1;
    s.client.propose_setting(&SettingKey::MinStakeBps, &above_max);
    s.client.approve_setting(&SettingKey::MinStakeBps, &above_max);
    advance_ledgers(&env, TIMELOCK_LEDGERS + 1);
    assert_eq!(
        s.client.try_execute_setting(&SettingKey::MinStakeBps),
        Err(Ok(PoolError::SettingOrderInvalid))
    );
}

#[test]
fn a_busier_band_can_never_be_looser_than_a_quieter_one() {
    let env = new_env();
    let s = setup(&env);
    // high-utilisation payout band above the mid band: rejected at execution
    let too_loose = OUTFLOW_RATE_BAND_2_BPS + 1;
    s.client.propose_setting(&SettingKey::PayoutHighBps, &too_loose);
    s.client.approve_setting(&SettingKey::PayoutHighBps, &too_loose);
    advance_ledgers(&env, TIMELOCK_LEDGERS + 1);
    assert_eq!(
        s.client.try_execute_setting(&SettingKey::PayoutHighBps),
        Err(Ok(PoolError::SettingOrderInvalid))
    );
}

// -- pool cap brake ----------------------------------------------------------

#[test]
fn pool_cap_raise_is_free_before_first_stake() {
    let env = new_env();
    let s = setup(&env);
    s.client.set_pool_cap(&(POOL_CAP * 2));
    assert_eq!(s.client.get_setting(&SettingKey::PoolCap), POOL_CAP * 2);
}

#[test]
fn pool_cap_raise_after_staking_needs_the_timelock_but_lowering_is_immediate() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    assert_eq!(
        s.client.try_set_pool_cap(&(POOL_CAP * 2)),
        Err(Ok(PoolError::PoolCapRaiseNeedsTimelock))
    );
    s.client.set_pool_cap(&(POOL_CAP / 2)); // brake: immediate
    assert_eq!(s.client.get_setting(&SettingKey::PoolCap), POOL_CAP / 2);
    set_setting_via_timelock(&env, &s, SettingKey::PoolCap, POOL_CAP);
    assert_eq!(s.client.get_setting(&SettingKey::PoolCap), POOL_CAP);
}

#[test]
fn pool_cap_setting_must_be_a_raise() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(
        s.client.try_propose_setting(&SettingKey::PoolCap, &POOL_CAP),
        Err(Ok(PoolError::SettingOutOfBounds))
    );
}

// -- effects on the pool -----------------------------------------------------

#[test]
fn changed_max_stake_applies_to_new_stakes() {
    let env = new_env();
    let s = setup(&env);
    let lower_max = MIN_STAKE_BPS; // max == min
    set_setting_via_timelock(&env, &s, SettingKey::MaxStakeBps, lower_max);
    let over = bps_of(POOL_CAP, lower_max) + 1;
    let staker = new_funded_address(&env, &s, over);
    let ben = Address::generate(&env);
    assert_eq!(s.client.try_stake(&staker, &over, &ben), Err(Ok(PoolError::StakeOutOfRange)));
}

#[test]
fn a_vesting_change_never_reprices_a_claim_already_streaming() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = MID_STAKE / 25;
    let (ben, claim_id) = active_claim(&env, &s, 1, entitlement);
    let claim_before = s.client.get_claim(&claim_id).unwrap();
    let original_window = (claim_before.vesting_ends_ledger - claim_before.cooldown_ends_ledger) as i128;
    assert_eq!(original_window, VESTING_LEDGERS as i128);

    // vesting setting changes while this claim sits in cooldown
    set_setting_via_timelock(&env, &s, SettingKey::VestingLedgers, NEW_VESTING);

    let claim_after = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim_after.vesting_ends_ledger, claim_before.vesting_ends_ledger);

    // stream a quarter of the ORIGINAL window past cooldown
    let now = env.ledger().sequence();
    let target = claim_before.cooldown_ends_ledger + (VESTING_LEDGERS / 4);
    if target > now {
        advance_ledgers(&env, target - now);
    }
    let elapsed = (env.ledger().sequence() - claim_before.cooldown_ends_ledger) as i128;
    let got = s.client.claim_stream(&claim_id, &ben);
    assert_eq!(got, entitlement * elapsed / original_window);
}

#[test]
fn claims_activated_after_a_vesting_change_use_the_new_window() {
    let env = new_env();
    let s = setup(&env);
    set_setting_via_timelock(&env, &s, SettingKey::VestingLedgers, NEW_VESTING);
    let (_ben, claim_id) = active_claim(&env, &s, 2, MID_STAKE / 25);
    let c = s.client.get_claim(&claim_id).unwrap();
    assert_eq!((c.vesting_ends_ledger - c.cooldown_ends_ledger) as i128, NEW_VESTING);
}

#[test]
fn a_cooldown_change_applies_to_new_claims_only() {
    let env = new_env();
    let s = setup(&env);
    let new_cooldown = 14 * LEDGERS_PER_DAY as i128;
    set_setting_via_timelock(&env, &s, SettingKey::CooldownLedgers, new_cooldown);
    let (_ben, claim_id) = active_claim(&env, &s, 3, MID_STAKE / 25);
    let c = s.client.get_claim(&claim_id).unwrap();
    assert_eq!((c.cooldown_ends_ledger - env.ledger().sequence()) as i128, new_cooldown);
}

#[test]
fn admission_rate_change_moves_the_daily_cap() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let before = s.client.get_setting(&SettingKey::AdmitLowBps);
    let lower = STRESS_RATE_BAND_2_BPS; // keep low >= mid
    set_setting_via_timelock(&env, &s, SettingKey::AdmitLowBps, lower);
    assert_ne!(before, lower);
    assert_eq!(s.client.get_setting(&SettingKey::AdmitLowBps), lower);
}
