//! v1 (2026-09-22): adjustable pool settings.
//!
//! Design decision: the pool is NOT upgradeable (no WASM swap, which could
//! change anything, including how funds move). Instead, a fixed list of
//! numbers can change, each only inside hard bounds written here, and only
//! through a public three-step path:
//!
//!   1. `propose_setting`  admin signs, value must be inside bounds
//!   2. `approve_setting`  co-signer signs the SAME value, starts a 7-day clock
//!   3. `execute_setting`  anyone, once the 7 days have passed
//!
//! Either signer can cancel before execution. Every step emits an event, so
//! stakers see a change coming a week ahead and can leave if they disagree
//! (this also closes T3's Repudiate.2 finding for this surface: admin
//! actions without events).
//!
//! What can NEVER change here: the 15/10/5 tier ceilings, the 90-day claim
//! gate, the 30-day claim window, and every funds-movement path. Those stay
//! compile-time constants.
//!
//! Settings read after a claim activates (cooldown, vesting) are snapshotted
//! onto the claim at activation (`claim.rs`), so a change only affects
//! claims that activate after it.

use soroban_sdk::{contractevent, contracttype, Address, Env};

use crate::error::PoolError;
use crate::storage;
use crate::types::{
    COOLDOWN_LEDGERS, LEDGERS_PER_DAY, MAX_STAKE_BPS, MIN_STAKE_BPS, SECONDS_PER_DAY,
    VESTING_LEDGERS, YIELD_SPLIT_STAKER_BPS,
};

/// Delay between co-signer approval and the earliest execution.
pub const SETTINGS_TIMELOCK_SECONDS: u64 = 7 * SECONDS_PER_DAY;

/// Every adjustable number. Discriminants are public ABI: append, never
/// renumber.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SettingKey {
    /// Min stake, bps of pool cap.
    MinStakeBps = 0,
    /// Max stake, bps of pool cap.
    MaxStakeBps = 1,
    /// Share of vault yield on STAKER capital paid to stakers, bps. The rest
    /// (and all yield on backer capital) goes to the protocol.
    StakerYieldBps = 2,
    /// Cooldown between claim activation and first payout, in ledgers.
    CooldownLedgers = 3,
    /// Linear vesting window after cooldown, in ledgers.
    VestingLedgers = 4,
    /// Admission daily cap, bps of capacity, by utilisation band (low/mid/high).
    AdmitLowBps = 5,
    AdmitMidBps = 6,
    AdmitHighBps = 7,
    /// Payout daily cap, bps of capacity, by utilisation band (low/mid/high).
    PayoutLowBps = 8,
    PayoutMidBps = 9,
    PayoutHighBps = 10,
    /// Backer withdrawal notice period, seconds.
    BackerNoticeSeconds = 11,
    /// Pool cap RAISE. Lowering stays immediate via `set_pool_cap` (a safety
    /// brake); raising after anyone has staked goes through this timelock.
    PoolCap = 12,
}

/// A proposed change waiting for co-signer approval and/or its timelock.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingSetting {
    pub value: i128,
    pub approved: bool,
    /// Earliest execution timestamp; 0 until approved.
    pub eta: u64,
}

// Hard bounds. Changing any of these means a new contract.
const STAKE_BPS_MIN: i128 = 1; // 0.01% of cap
const STAKE_BPS_MAX: i128 = 125; // 1.25% of cap (T3's max)
const STAKER_YIELD_BPS_MAX: i128 = 10_000; // 100%
const COOLDOWN_MIN: i128 = 7 * LEDGERS_PER_DAY as i128;
const COOLDOWN_MAX: i128 = 30 * LEDGERS_PER_DAY as i128;
const VESTING_MIN: i128 = 30 * LEDGERS_PER_DAY as i128;
const VESTING_MAX: i128 = 90 * LEDGERS_PER_DAY as i128;
const ADMIT_BPS_MIN: i128 = 1;
const ADMIT_BPS_MAX: i128 = 2_500; // 25%/day, today's highest band
const PAYOUT_BPS_MIN: i128 = 1;
const PAYOUT_BPS_MAX: i128 = 600; // 6%/day ceiling on money leaving
const BACKER_NOTICE_MAX: i128 = 90 * SECONDS_PER_DAY as i128;

/// Defaults = the values the pool shipped with, so a fresh deploy behaves
/// exactly as before until a setting is changed.
fn default_value(env: &Env, key: SettingKey) -> i128 {
    match key {
        SettingKey::MinStakeBps => MIN_STAKE_BPS,
        SettingKey::MaxStakeBps => MAX_STAKE_BPS,
        SettingKey::StakerYieldBps => YIELD_SPLIT_STAKER_BPS,
        SettingKey::CooldownLedgers => COOLDOWN_LEDGERS as i128,
        SettingKey::VestingLedgers => VESTING_LEDGERS as i128,
        SettingKey::AdmitLowBps => 2_500,
        SettingKey::AdmitMidBps => 1_000,
        SettingKey::AdmitHighBps => 300,
        SettingKey::PayoutLowBps => 500,
        SettingKey::PayoutMidBps => 300,
        SettingKey::PayoutHighBps => 100,
        SettingKey::BackerNoticeSeconds => 30 * SECONDS_PER_DAY as i128,
        SettingKey::PoolCap => storage::get_pool_cap(env),
    }
}

fn in_bounds(env: &Env, key: SettingKey, value: i128) -> bool {
    match key {
        SettingKey::MinStakeBps | SettingKey::MaxStakeBps => {
            (STAKE_BPS_MIN..=STAKE_BPS_MAX).contains(&value)
        }
        SettingKey::StakerYieldBps => (0..=STAKER_YIELD_BPS_MAX).contains(&value),
        SettingKey::CooldownLedgers => (COOLDOWN_MIN..=COOLDOWN_MAX).contains(&value),
        SettingKey::VestingLedgers => (VESTING_MIN..=VESTING_MAX).contains(&value),
        SettingKey::AdmitLowBps | SettingKey::AdmitMidBps | SettingKey::AdmitHighBps => {
            (ADMIT_BPS_MIN..=ADMIT_BPS_MAX).contains(&value)
        }
        SettingKey::PayoutLowBps | SettingKey::PayoutMidBps | SettingKey::PayoutHighBps => {
            (PAYOUT_BPS_MIN..=PAYOUT_BPS_MAX).contains(&value)
        }
        SettingKey::BackerNoticeSeconds => (0..=BACKER_NOTICE_MAX).contains(&value),
        SettingKey::PoolCap => value > storage::get_pool_cap(env),
    }
}

/// Cross-setting invariants, checked against LIVE values at execution:
/// min stake <= max stake, and each rate band no looser than the band below
/// it in utilisation (low >= mid >= high).
fn consistent_with_live(env: &Env, key: SettingKey, value: i128) -> bool {
    let g = |k| get(env, k);
    match key {
        SettingKey::MinStakeBps => value <= g(SettingKey::MaxStakeBps),
        SettingKey::MaxStakeBps => value >= g(SettingKey::MinStakeBps),
        SettingKey::AdmitLowBps => value >= g(SettingKey::AdmitMidBps),
        SettingKey::AdmitMidBps => {
            value <= g(SettingKey::AdmitLowBps) && value >= g(SettingKey::AdmitHighBps)
        }
        SettingKey::AdmitHighBps => value <= g(SettingKey::AdmitMidBps),
        SettingKey::PayoutLowBps => value >= g(SettingKey::PayoutMidBps),
        SettingKey::PayoutMidBps => {
            value <= g(SettingKey::PayoutLowBps) && value >= g(SettingKey::PayoutHighBps)
        }
        SettingKey::PayoutHighBps => value <= g(SettingKey::PayoutMidBps),
        _ => true,
    }
}

/// Live value of a setting (its default until first changed).
pub fn get(env: &Env, key: SettingKey) -> i128 {
    if key == SettingKey::PoolCap {
        return storage::get_pool_cap(env);
    }
    storage::get_setting(env, key).unwrap_or_else(|| default_value(env, key))
}

pub fn get_pending(env: &Env, key: SettingKey) -> Option<PendingSetting> {
    storage::get_pending_setting(env, key)
}

// -----------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------

#[contractevent]
pub struct SettingProposed {
    #[topic]
    pub key: SettingKey,
    pub value: i128,
}

#[contractevent]
pub struct SettingApproved {
    #[topic]
    pub key: SettingKey,
    pub value: i128,
    pub eta: u64,
}

#[contractevent]
pub struct SettingExecuted {
    #[topic]
    pub key: SettingKey,
    pub old_value: i128,
    pub new_value: i128,
}

#[contractevent]
pub struct SettingCancelled {
    #[topic]
    pub key: SettingKey,
    pub by: Address,
}

// -----------------------------------------------------------------------
// Three-step change path
// -----------------------------------------------------------------------

/// Step 1: admin proposes. Replaces any earlier, not-yet-executed proposal
/// for the same key (which also resets its approval and clock).
pub fn propose_setting(env: &Env, key: SettingKey, value: i128) -> Result<(), PoolError> {
    storage::get_admin(env).require_auth();
    if !in_bounds(env, key, value) {
        return Err(PoolError::SettingOutOfBounds);
    }
    storage::set_pending_setting(
        env,
        key,
        &PendingSetting { value, approved: false, eta: 0 },
    );
    storage::bump_instance_ttl(env);
    SettingProposed { key, value }.publish(env);
    Ok(())
}

/// Step 2: co-signer approves the exact same value; starts the 7-day clock.
pub fn approve_setting(env: &Env, key: SettingKey, value: i128) -> Result<(), PoolError> {
    storage::get_co_signer(env).require_auth();
    let mut pending = storage::get_pending_setting(env, key).ok_or(PoolError::NoPendingSetting)?;
    if pending.value != value {
        return Err(PoolError::SettingValueMismatch);
    }
    if pending.approved {
        return Err(PoolError::SettingAlreadyApproved);
    }
    pending.approved = true;
    pending.eta = env.ledger().timestamp() + SETTINGS_TIMELOCK_SECONDS;
    storage::set_pending_setting(env, key, &pending);
    storage::bump_instance_ttl(env);
    SettingApproved { key, value, eta: pending.eta }.publish(env);
    Ok(())
}

/// Step 3: anyone executes once approved and the timelock has passed.
/// Bounds and cross-setting invariants are re-checked against live state.
pub fn execute_setting(env: &Env, key: SettingKey) -> Result<(), PoolError> {
    let pending = storage::get_pending_setting(env, key).ok_or(PoolError::NoPendingSetting)?;
    if !pending.approved || env.ledger().timestamp() < pending.eta {
        return Err(PoolError::SettingNotReady);
    }
    if !in_bounds(env, key, pending.value) {
        return Err(PoolError::SettingOutOfBounds);
    }
    if !consistent_with_live(env, key, pending.value) {
        return Err(PoolError::SettingOrderInvalid);
    }
    let old_value = get(env, key);
    if key == SettingKey::PoolCap {
        storage::set_pool_cap(env, pending.value);
    } else {
        storage::set_setting(env, key, pending.value);
    }
    storage::remove_pending_setting(env, key);
    storage::bump_instance_ttl(env);
    SettingExecuted { key, old_value, new_value: pending.value }.publish(env);
    Ok(())
}

/// Either signer can cancel a pending change before it executes.
pub fn cancel_setting(env: &Env, caller: &Address, key: SettingKey) -> Result<(), PoolError> {
    caller.require_auth();
    if caller != &storage::get_admin(env) && caller != &storage::get_co_signer(env) {
        return Err(PoolError::CallerNotAdminOrCoSigner);
    }
    if storage::get_pending_setting(env, key).is_none() {
        return Err(PoolError::NoPendingSetting);
    }
    storage::remove_pending_setting(env, key);
    storage::bump_instance_ttl(env);
    SettingCancelled { key, by: caller.clone() }.publish(env);
    Ok(())
}
