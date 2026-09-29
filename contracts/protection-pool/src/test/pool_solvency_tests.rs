#![cfg(test)]
//! v1 (2026-09-22): pool solvency under the pool's liquidity model
//! ("stake inflow > payouts": stakers are always repaid in full, nobody is
//! deducted; a claim larger than the claimant's stake is funded from pooled
//! liquidity).
//!
//! Scenario (design numbers): $100K pool cap, filled by 1,000 stakers at
//! the $100 max stake, 80% deployed to the vault, 5% yearly vault yield,
//! yield split 50% stakers / 50% protocol, protocol share left in the pool.
//!
//! What the numbers mean:
//! - The only money nobody can ask back is the protocol's retained yield
//!   plus each claimant's forfeited stake (and its yield share).
//! - Each paid claim drains `entitlement - forfeited value` from that buffer.
//! - Claims up to `buffer / drain` are absorbed with every remaining staker
//!   still repaid in full. One more and the last withdrawer must wait for new
//!   stake: the by-design edge of the model, and new stake then clears it.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use super::common::*;
use super::d2_vault_tests::with_vault;
use crate::error::PoolError;
use crate::settings::SettingKey;
use crate::types::{BPS_DENOMINATOR, COOLDOWN_LEDGERS, MAX_DEPLOY_BPS, VESTING_LEDGERS};

const USD: i128 = STROOPS_PER_UNIT;
const CAP: i128 = 100_000 * USD;
const STAKE: i128 = 100 * USD; // max stake = 0.1% of cap
const STAKERS: usize = 1_000;
const VAULT_APY_BPS: i128 = 500; // 5% a year
const STAKER_SHARE_BPS: i128 = 5_000; // 50/50
const TIER_C: u32 = 3;
const TIER_C_MULTIPLE: i128 = 5;

struct Outcome {
    realised_yield: i128,
    protocol_buffer: i128,
    drain_per_claim: i128,
    full: usize,
    stuck: std::vec::Vec<(Address, Address)>,
}

fn run(env: &Env, claims: usize) -> (Setup<'_>, Outcome) {
    env.cost_estimate().budget().reset_unlimited();
    let s = setup_with_cap(env, CAP);
    set_setting_via_timelock(env, &s, SettingKey::StakerYieldBps, STAKER_SHARE_BPS);

    let mut stakers = std::vec::Vec::with_capacity(STAKERS);
    for _ in 0..STAKERS {
        stakers.push(staked_wallet_amount(env, &s, STAKE));
    }
    assert_eq!(s.client.get_total_staked(), CAP);

    // One year in the vault at 5%.
    let (vault_id, mock) = with_vault(env, &s, MAX_DEPLOY_BPS);
    let deployed = bps_of(CAP, MAX_DEPLOY_BPS);
    s.client.deploy_to_vault(&deployed, &0);
    advance_days(env, 365);
    mock.set_rate_bps(&(BPS_DENOMINATOR + VAULT_APY_BPS));
    s.token_admin.mint(&vault_id, &bps_of(deployed, VAULT_APY_BPS));
    let realised_yield = s.client.extract_yield(&s.client.get_total_deployed_shares(), &0);
    let protocol_buffer = s.client.get_yield_balance(); // left in the pool

    // Claims: 5x the stake, Tier C, paid in full.
    let entitlement = STAKE * TIER_C_MULTIPLE;
    let mut active = std::vec::Vec::new();
    for (i, (staker, ben)) in stakers.iter().take(claims).enumerate() {
        let id = submit_claim_signed(
            env, &s, &s.oracle, staker, &tx_hash(env, (i % 250) as u8 + 1), &entitlement, &TIER_C, &now_ts(env),
        );
        s.client.approve_claim(&id);
        active.push((id, ben.clone()));
    }
    advance_ledgers(env, COOLDOWN_LEDGERS + VESTING_LEDGERS);
    for (id, ben) in &active {
        let mut paid = 0;
        for _ in 0..30 {
            if let Ok(Ok(a)) = s.client.try_claim_stream(id, ben) {
                paid += a;
            }
            if paid >= entitlement {
                break;
            }
            advance_days(env, 1);
        }
        assert_eq!(paid, entitlement, "every admitted claim is paid in full");
    }

    // A claimant's forfeited value = its stake plus the yield share it accrued.
    let forfeited_value = s.client.get_withdrawable_amount(&stakers[claims.min(STAKERS - 1)].0);
    let drain_per_claim = entitlement - forfeited_value;

    // Everyone else withdraws.
    let mut full = 0;
    let mut stuck = std::vec::Vec::new();
    for (staker, ben) in stakers.iter().skip(claims) {
        match s.client.try_withdraw(staker, ben) {
            Ok(Ok(())) => full += 1,
            Err(Ok(PoolError::InsufficientLiquidity)) => stuck.push((staker.clone(), ben.clone())),
            other => panic!("unexpected withdraw result: {:?}", other),
        }
    }
    (s, Outcome { realised_yield, protocol_buffer, drain_per_claim, full, stuck })
}

fn max_absorbed(o: &Outcome) -> usize {
    (o.protocol_buffer / o.drain_per_claim) as usize
}

#[test]
fn solvency_no_claims_everyone_repaid_with_yield() {
    let env = new_env();
    let (s, o) = run(&env, 0);
    std::println!(
        "[0 claims] realised yield ${} | protocol buffer ${} | withdrew in full {}/{} | stuck {}",
        o.realised_yield / USD, o.protocol_buffer / USD, o.full, STAKERS, o.stuck.len()
    );
    assert_eq!(o.realised_yield, bps_of(bps_of(CAP, MAX_DEPLOY_BPS), VAULT_APY_BPS));
    assert_eq!(o.protocol_buffer, o.realised_yield - bps_of(o.realised_yield, STAKER_SHARE_BPS));
    assert_eq!(o.full, STAKERS);
    assert!(o.stuck.is_empty());
    // what is left is exactly the protocol's share, withdrawable in full
    assert_eq!(s.client.get_liquid_balance(), o.protocol_buffer);
}

#[test]
fn solvency_claims_up_to_the_buffer_are_absorbed_in_full() {
    let env = new_env();
    let (_s0, probe) = run(&env, 0);
    let k = max_absorbed(&Outcome { stuck: std::vec::Vec::new(), ..probe });
    let env = new_env();
    let (_s, o) = run(&env, k);
    std::println!(
        "[{} claims x $500] drain per claim ${:.2} | buffer ${} | withdrew in full {}/{} | stuck {}",
        k, o.drain_per_claim as f64 / USD as f64, o.protocol_buffer / USD, o.full, STAKERS - k, o.stuck.len()
    );
    assert!(k >= 1);
    assert!(o.stuck.is_empty(), "{} claims must be fully absorbed", k);
}

#[test]
fn solvency_one_claim_past_the_buffer_makes_the_last_withdrawer_wait_until_new_stake() {
    let env = new_env();
    let (_s0, probe) = run(&env, 0);
    let k = max_absorbed(&Outcome { stuck: std::vec::Vec::new(), ..probe }) + 1;
    let env = new_env();
    let (s, o) = run(&env, k);
    std::println!(
        "[{} claims x $500] withdrew in full {}/{} | stuck {} (waiting for new stake)",
        k, o.full, STAKERS - k, o.stuck.len()
    );
    assert!(!o.stuck.is_empty(), "past the buffer, the last withdrawer must wait");

    // The liquidity model: new stake arrives and the stuck staker is repaid in full.
    let newcomer = new_funded_address(&env, &s, STAKE);
    s.client.stake(&newcomer, &STAKE, &Address::generate(&env));
    let (staker, ben) = &o.stuck[0];
    s.client.withdraw(staker, ben);
}
