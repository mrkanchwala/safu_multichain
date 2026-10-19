//! v1 (2026-09-22): backers.
//!
//! A backer deposits money that adds to the pool's capacity and liquidity
//! (a confidence signal for stakers). Anyone can back. A staker's deposit
//! above the max stake also lands here.
//!
//! r3 (2026-09-29, design decision): backers earn yield on COUNTED money
//! (`BackerYieldBps`, default 100%; the rest goes to the protocol). Money
//! still waiting to count earns nothing. Yield can be taken any time
//! (`claim_backer_yield`), with no free-capital check, and is also paid with
//! every withdrawal payment.
//!
//! No loss accounting (design decision 2026-09-22, same as stakers): a
//! backer always gets back exactly what they put in. What makes backers the
//! last line of defence is rule 3 below: a staker can leave with only a
//! liquidity check, a backer can only take out capital that open claims do
//! not need.
//!
//! The withdrawal-safety rules (locked 2026-09-22, revised 2026-10-16):
//!   1. Money returns only to the depositing address, only with that
//!      address's signature. No admin function touches backer money.
//!   2. Money requested out keeps counting toward capacity until it is paid.
//!   3. Only free capital can leave: after a payment,
//!      `total_allocated <= capacity`.
//!   4. New money counts after `BackerMaturitySecs` (default 0: from day 1).
//!      Each deposit keeps its own clock.
//!   5. A withdrawal drips out daily instead of waiting
//!      out a notice. The drip takes `DripMinSecs` (7 d) on an idle pool up to
//!      `DripMaxSecs` (90 d) at >= 50% utilisation, recomputed each day.
//!      Each day every open request gets a pro-rata share (by what it is still
//!      owed) of that day's free capital, so collecting first takes nothing
//!      from the others. A day on which open claims need every dollar pays
//!      nothing; the drip resumes once capital frees up.
//!
//! Pause: new deposits are blocked; requesting, cancelling and collecting a
//! withdrawal still work (rule 3 still applies), so a pause can never be used
//! to hold backer money.

use soroban_sdk::{contractevent, token::TokenClient, vec, Address, Env};

use crate::error::PoolError;
use crate::settings::{self, SettingKey};
use crate::storage;
use crate::types::{
    BackerRecord, PendingDeposit, BPS_DENOMINATOR, DRIP_FULL_UTIL_BPS, MAX_PENDING_DEPOSITS,
    SECONDS_PER_DAY, YIELD_INDEX_PRECISION,
};

#[contractevent]
pub struct Backed {
    #[topic]
    pub backer: Address,
    pub amount: i128,
    pub matures_at: u64,
}

#[contractevent]
pub struct BackingMatured {
    #[topic]
    pub backer: Address,
    pub amount: i128,
}

#[contractevent]
pub struct BackerWithdrawalRequested {
    #[topic]
    pub backer: Address,
    pub amount: i128,
    /// Estimate of the last drip day at today's utilisation.
    pub ready_at: u64,
}

#[contractevent]
pub struct BackerWithdrawalCancelled {
    #[topic]
    pub backer: Address,
    /// The unpaid part that stays in the pool.
    pub amount: i128,
}

#[contractevent]
pub struct BackerWithdrawn {
    #[topic]
    pub backer: Address,
    /// Principal paid by this collection.
    pub amount: i128,
}

fn empty_record(env: &Env) -> BackerRecord {
    BackerRecord {
        amount: 0,
        pending: vec![env],
        withdraw_amount: 0,
        withdraw_paid: 0,
        withdraw_started_at: 0,
        share_day: 0,
        share_left: 0,
        yield_index_at: storage::get_backer_yield_index(env),
        yield_owed: 0,
        points_index_at: crate::points::index(env, crate::points::BACKER),
        points: 0,
    }
}

fn today(env: &Env) -> u32 {
    (env.ledger().timestamp() / SECONDS_PER_DAY) as u32
}

/// r3: settles yield earned on the counted balance since the last
/// settlement into `yield_owed`. Must run before `amount` changes.
fn accrue_yield(env: &Env, record: &mut BackerRecord) {
    let index = storage::get_backer_yield_index(env);
    let delta = index - record.yield_index_at;
    if delta > 0 && record.amount > 0 {
        record.yield_owed += record.amount * delta / YIELD_INDEX_PRECISION;
    }
    record.yield_index_at = index;
    // Points on the counted balance, settled the same way.
    let p = crate::points::index(env, crate::points::BACKER);
    record.points += crate::points::earned(record.amount, record.points_index_at, p);
    record.points_index_at = p;
}

/// Moves every matured deposit into the counted balance. Returns the amount
/// moved. Caller persists `record`. r3: settles yield first, so newly
/// counted money earns only from now.
fn settle_maturity(env: &Env, record: &mut BackerRecord) -> i128 {
    accrue_yield(env, record);
    let now = env.ledger().timestamp();
    let mut moved = 0;
    let mut waiting = vec![env];
    for d in record.pending.iter() {
        if now >= d.matures_at {
            moved += d.amount;
        } else {
            waiting.push_back(d);
        }
    }
    if moved > 0 {
        record.pending = waiting;
        record.amount += moved;
        storage::set_total_backed_pending(env, storage::get_total_backed_pending(env) - moved);
        storage::set_total_backed(env, storage::get_total_backed(env) + moved);
    }
    moved
}

/// Books `amount` of backing for `backer`. No auth, no harvest, no
/// transfer: the caller (`back`, or `stake` for a deposit above the max
/// stake) has done the first two and does the transfer after. Shared so a
/// stake overflow and a direct backing follow exactly the same rules.
pub(crate) fn add_backing(env: &Env, backer: &Address, amount: i128) -> Result<(), PoolError> {
    crate::points::checkpoint(env, crate::points::BACKER);
    let mut record = storage::get_backer(env, backer).unwrap_or_else(|| empty_record(env));
    let matured = settle_maturity(env, &mut record);

    let now = env.ledger().timestamp();
    let matures_at = now + settings::get(env, SettingKey::BackerMaturitySecs) as u64;
    if matures_at <= now {
        // Rule 11 default: counts (and earns) from now.
        record.amount += amount;
        storage::set_total_backed(env, storage::get_total_backed(env) + amount);
    } else {
        if record.pending.len() >= MAX_PENDING_DEPOSITS {
            return Err(PoolError::TooManyPendingDeposits);
        }
        record.pending.push_back(PendingDeposit { amount, matures_at });
        storage::set_total_backed_pending(env, storage::get_total_backed_pending(env) + amount);
    }
    storage::set_backer(env, backer, &record);
    // r3: from the first backing on, vault/treasury/deploy-ceiling changes wait 7 days.
    storage::set_ever_funded(env);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    Backed { backer: backer.clone(), amount, matures_at }.publish(env);
    Ok(())
}

/// Deposit.
pub fn back(env: &Env, backer: &Address, amount: i128) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;
    backer.require_auth();
    if amount <= 0 {
        return Err(PoolError::AmountNotPositive);
    }

    // r3 (code review B2): growth so far goes to money already counted,
    // before any new money joins `total_backed`. Best effort.
    crate::vault::harvest(env);
    add_backing(env, backer, amount)?;

    // Interaction last (CEI): backer to contract.
    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(backer, env.current_contract_address(), &amount);
    // v1: put idle cash above the buffer into the vault. Never fails the deposit.
    crate::vault::push_idle(env);
    Ok(())
}

/// Permissionless: start counting every matured deposit. Until someone
/// calls this (or the backer acts), capacity is undercounted (the safe
/// direction). Only matters when `BackerMaturitySecs` > 0.
pub fn mature_backing(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.pending.is_empty() {
        return Err(PoolError::NoPendingBacking);
    }
    // r3 (code review B2): growth so far goes to money already counted.
    crate::vault::harvest(env);
    let moved = settle_maturity(env, &mut record);
    if moved == 0 {
        return Err(PoolError::BackingNotMature);
    }
    storage::set_backer(env, backer, &record);
    storage::bump_instance_ttl(env);
    BackingMatured { backer: backer.clone(), amount: moved }.publish(env);
    // v1: capacity just grew, so the vault line moved: push idle cash.
    crate::vault::push_idle(env);
    Ok(moved)
}

// -----------------------------------------------------------------------
// The daily drip
// -----------------------------------------------------------------------

/// (free capital, open requests, utilisation bps) from live state.
fn snap_values(env: &Env) -> (i128, i128, i128) {
    let capacity = storage::get_capacity(env);
    let allocated = storage::get_total_allocated(env);
    let util = if capacity > 0 { allocated * BPS_DENOMINATOR / capacity } else { 0 };
    ((capacity - allocated).max(0), storage::get_total_backer_open(env), util)
}

/// The day's drip snapshot, taken once, at the day's first backer action.
/// `store = false` reads it without writing (views).
fn drip_snap(env: &Env, day: u32, store: bool) -> (i128, i128, i128) {
    if let Some((d, free, open, util)) = storage::get_drip_snap(env) {
        if d == day {
            return (free, open, util);
        }
    }
    let v = snap_values(env);
    if store {
        storage::set_drip_snap(env, &(day, v.0, v.1, v.2));
    }
    v
}

/// Drip length at a utilisation, in whole days (at least 1).
pub(crate) fn drip_days(env: &Env, util_bps: i128) -> i128 {
    let lo = settings::get(env, SettingKey::DripMinSecs);
    let hi = settings::get(env, SettingKey::DripMaxSecs);
    let u = util_bps.clamp(0, DRIP_FULL_UTIL_BPS);
    let secs = lo + (hi - lo) * u / DRIP_FULL_UTIL_BPS;
    let day = SECONDS_PER_DAY as i128;
    ((secs + day - 1) / day).max(1)
}

/// The three limits on a collection right now, before the liquidity pull:
/// (what the drip has unlocked and not yet paid, this backer's share of the
/// day's free capital, live free capital for rule 3). Sets the record's
/// day share in memory when the day has turned.
fn limits(env: &Env, record: &mut BackerRecord, store: bool) -> (i128, i128, i128, i128) {
    let day = today(env);
    let (snap_free, snap_open, util) = drip_snap(env, day, store);
    let elapsed = ((env.ledger().timestamp() - record.withdraw_started_at) / SECONDS_PER_DAY) as i128;
    let unlocked = (record.withdraw_amount * (elapsed + 1) / drip_days(env, util)).min(record.withdraw_amount);
    if record.share_day != day {
        // A request made after the snapshot isn't in `snap_open`; treat its
        // own remainder as the whole (same pattern as the claim day share).
        let remaining = record.withdraw_amount - record.withdraw_paid;
        let owed_all = snap_open.max(remaining);
        record.share_left = if owed_all > 0 { (snap_free * remaining + owed_all - 1) / owed_all } else { 0 };
        record.share_day = day;
    }
    let free_now = storage::get_capacity(env) - storage::get_total_allocated(env);
    (unlocked - record.withdraw_paid, record.share_left, free_now, snap_free)
}

/// What can leave on this call: the smallest of the three limits. Errors
/// say which limit is hit.
fn collectable(env: &Env, record: &mut BackerRecord) -> Result<i128, PoolError> {
    let (due, share, free_now, snap_free) = limits(env, record, true);
    if due <= 0 {
        return Err(PoolError::NothingToCollectYet);
    }
    if snap_free <= 0 || free_now <= 0 {
        return Err(PoolError::BackerCapitalNotFree);
    }
    let pay = due.min(share).min(free_now);
    if pay <= 0 {
        return Err(PoolError::NothingToCollectYet);
    }
    Ok(pay)
}

/// Opens a withdrawal request. Only counted money can be requested; it keeps
/// counting until paid (rule 2). Returns an estimate of when the last drip
/// day falls at today's utilisation. Works while paused.
pub fn request_withdrawal(env: &Env, backer: &Address, amount: i128) -> Result<u64, PoolError> {
    backer.require_auth();
    if amount <= 0 {
        return Err(PoolError::AmountNotPositive);
    }
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.withdraw_amount > 0 {
        return Err(PoolError::BackerWithdrawalPending);
    }
    let matured = settle_maturity(env, &mut record);
    if amount > record.amount {
        return Err(PoolError::BackerAmountExceedsBalance);
    }

    let now = env.ledger().timestamp();
    let (_, _, util) = drip_snap(env, today(env), true);
    let ready_at = now + (drip_days(env, util) as u64 - 1) * SECONDS_PER_DAY;
    record.withdraw_amount = amount;
    record.withdraw_paid = 0;
    record.withdraw_started_at = now;
    record.share_day = 0;
    record.share_left = 0;
    storage::set_backer(env, backer, &record);
    storage::set_total_backer_open(env, storage::get_total_backer_open(env) + amount);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    BackerWithdrawalRequested { backer: backer.clone(), amount, ready_at }.publish(env);
    Ok(ready_at)
}

fn clear_request(record: &mut BackerRecord) {
    record.withdraw_amount = 0;
    record.withdraw_paid = 0;
    record.withdraw_started_at = 0;
    record.share_day = 0;
    record.share_left = 0;
}

/// Backer stops the drip; the unpaid part stays in the pool. Works while paused.
pub fn cancel_withdrawal(env: &Env, backer: &Address) -> Result<(), PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.withdraw_amount <= 0 {
        return Err(PoolError::NoBackerWithdrawal);
    }
    let unpaid = record.withdraw_amount - record.withdraw_paid;
    clear_request(&mut record);
    storage::set_backer(env, backer, &record);
    storage::set_total_backer_open(env, storage::get_total_backer_open(env) - unpaid);
    storage::bump_instance_ttl(env);
    BackerWithdrawalCancelled { backer: backer.clone(), amount: unpaid }.publish(env);
    Ok(())
}

/// Collects what the drip allows today (rule 5), plus any unpaid yield.
/// On failure nothing changes; the backer retries later. Pays only the
/// backer's own address (rule 1). Works while paused. Returns the principal
/// paid by this call; the request closes once all of it is paid.
pub fn complete_withdrawal(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.withdraw_amount <= 0 {
        return Err(PoolError::NoBackerWithdrawal);
    }
    let pay = collectable(env, &mut record)?;
    crate::vault::harvest(env);

    let matured = settle_maturity(env, &mut record);
    // Unpaid yield goes out with the principal. It is pool cash
    // until paid, so it may need pulling from the vault too.
    let paid_yield = crate::vault::take_backer_yield(env, record.yield_owed);
    crate::vault::pull_for_payment(env, pay + paid_yield)?;

    // Effects before interaction (CEI).
    record.yield_owed -= paid_yield;
    record.amount -= pay;
    record.withdraw_paid += pay;
    record.share_left -= pay;
    if record.withdraw_paid >= record.withdraw_amount {
        clear_request(&mut record);
    }
    storage::set_backer(env, backer, &record);
    storage::set_total_backed(env, storage::get_total_backed(env) - pay);
    storage::set_total_backer_open(env, storage::get_total_backer_open(env) - pay);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    BackerWithdrawn { backer: backer.clone(), amount: pay }.publish(env);
    if paid_yield > 0 {
        crate::vault::YieldPaid { owner: backer.clone(), to: backer.clone(), amount: paid_yield }.publish(env);
    }

    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(&env.current_contract_address(), backer, &(pay + paid_yield));
    Ok(pay)
}

/// Read-only: what `complete_withdrawal` would pay right now, before the
/// liquidity pull (0 if nothing). Writes nothing.
pub fn collectable_view(env: &Env, backer: &Address) -> i128 {
    match storage::get_backer(env, backer) {
        Some(mut r) if r.withdraw_amount > 0 => {
            let (due, share, free_now, _) = limits(env, &mut r, false);
            due.min(share).min(free_now).max(0)
        }
        _ => 0,
    }
}

/// r3 (2026-09-29): take backer yield any time, no free-capital check,
/// principal untouched. Paid only to the backer's own address (rule 1).
/// Works while paused (harvest skipped then). Also matures any pending
/// money that is due, so it starts earning.
pub fn claim_yield(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    crate::vault::harvest(env);
    let matured = settle_maturity(env, &mut record);
    let paid = crate::vault::take_backer_yield(env, record.yield_owed);
    if paid <= 0 {
        return Err(PoolError::NoYieldOwed);
    }
    crate::vault::pull_for_payment(env, paid)?;
    record.yield_owed -= paid;
    storage::set_backer(env, backer, &record);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    crate::vault::YieldPaid { owner: backer.clone(), to: backer.clone(), amount: paid }.publish(env);

    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(&env.current_contract_address(), backer, &paid);
    Ok(paid)
}

/// r3: view of a backer's unpaid yield, including what has accrued since
/// the record was last touched. Read-only.
pub fn points_view(env: &Env, backer: &Address) -> i128 {
    match storage::get_backer(env, backer) {
        Some(mut record) => {
            accrue_yield(env, &mut record);
            record.points
        }
        None => 0,
    }
}

pub fn yield_owed_view(env: &Env, backer: &Address) -> i128 {
    match storage::get_backer(env, backer) {
        Some(mut record) => {
            accrue_yield(env, &mut record);
            record.yield_owed
        }
        None => 0,
    }
}
