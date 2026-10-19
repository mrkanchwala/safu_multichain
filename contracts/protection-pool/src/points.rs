//! Points are dollar-days. 1 point per
//! whole unit per day at the default rate, with separate staker and backer
//! rate settings. Each side keeps a points index (points per unit so far),
//! so a rate change applies from that moment on and never rescales points
//! already earned. Replaces the age-band formula, which divided by the live
//! max stake (a max change rescaled everyone's points).

use soroban_sdk::Env;

use crate::settings::{self, SettingKey};
use crate::storage;
use crate::types::SECONDS_PER_DAY;

/// Rate setting value for 1 point per unit per day.
pub const POINTS_RATE_ONE: i128 = 10_000;
/// Stroops per whole unit of the pool asset (7 decimals).
const UNIT: i128 = 10_000_000;

pub const STAKER: u32 = 0;
pub const BACKER: u32 = 1;

fn rate_key(side: u32) -> SettingKey {
    if side == STAKER {
        SettingKey::PointsRateStaker
    } else {
        SettingKey::PointsRateBacker
    }
}

/// The side's index right now: the stored value plus the live rate times
/// the seconds since it was stored. 0 until the side's first checkpoint.
pub(crate) fn index(env: &Env, side: u32) -> i128 {
    let (stored, at) = storage::get_points_index(env, side);
    if at == 0 {
        return stored;
    }
    let elapsed = env.ledger().timestamp().saturating_sub(at) as i128;
    stored + settings::get(env, rate_key(side)) * elapsed
}

/// Stores the index as of now. Called before a rate change (so the old
/// rate covers the time before it) and on every stake and backing (so the
/// clock is running before anyone holds money on that side).
pub(crate) fn checkpoint(env: &Env, side: u32) {
    storage::set_points_index(env, side, index(env, side), env.ledger().timestamp().max(1));
}

/// Whole points earned by `amount` between two index values.
pub(crate) fn earned(amount: i128, from: i128, to: i128) -> i128 {
    if amount <= 0 || to <= from {
        return 0;
    }
    amount * (to - from) / (UNIT * POINTS_RATE_ONE * SECONDS_PER_DAY as i128)
}

/// The side a points-rate setting belongs to, if `key` is one.
pub(crate) fn side_of(key: SettingKey) -> Option<u32> {
    match key {
        SettingKey::PointsRateStaker => Some(STAKER),
        SettingKey::PointsRateBacker => Some(BACKER),
        _ => None,
    }
}
