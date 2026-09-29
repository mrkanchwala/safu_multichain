//! v1 (2026-09-22): backers.
//!
//! A backer deposits money that adds to the pool's capacity and liquidity
//! (a confidence signal for stakers). Anyone can back.
//!
//! r3 (2026-09-29, design decision): backers earn yield on MATURED money
//! (`BackerYieldBps`, default 100%; the rest goes to the protocol). Money in
//! its maturity wait earns nothing. Yield can be taken any time
//! (`claim_backer_yield`), with no notice and no free-capital check, and is
//! also paid with every completed withdrawal. Before r3 all yield on backer
//! money went to the protocol.
//!
//! No loss accounting (design decision 2026-09-22, same as stakers): a
//! backer always gets back exactly what they put in. What makes backers the
//! last line of defence is rule 3 below: a staker can leave with only a
//! liquidity check, a backer can only take out capital that open claims do
//! not need.
//!
//! The four withdrawal-safety rules (locked 2026-09-22):
//!   1. Money returns only to the depositing address, only with that
//!      address's signature. No admin function touches backer money.
//!   2. During the notice period the money still counts toward capacity.
//!   3. Only free capital can leave: after the withdrawal,
//!      `total_allocated <= capacity`.
//!   4. New money counts toward capacity only after `BACKER_MATURITY_SECONDS`,
//!      which blocks deposit spike -> admit claims -> pull out.
//!
//! Pause: new deposits are blocked; requesting, cancelling and completing a
//! withdrawal still work (rule 3 still applies), so a pause can never be used
//! to hold backer money.
//!
//! Fixed in code: the maturity period, full-amount return, anyone-can-back,
//! pause behaviour. Adjustable: only the notice period
//! (`SettingKey::BackerNoticeSeconds`, settings.rs).

use soroban_sdk::{contractevent, token::TokenClient, Address, Env};

use crate::error::PoolError;
use crate::storage;
use crate::types::{BackerRecord, BACKER_MATURITY_SECONDS, YIELD_INDEX_PRECISION};

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
    pub ready_at: u64,
}

#[contractevent]
pub struct BackerWithdrawalCancelled {
    #[topic]
    pub backer: Address,
    pub amount: i128,
}

#[contractevent]
pub struct BackerWithdrawn {
    #[topic]
    pub backer: Address,
    pub amount: i128,
}

fn empty_record(env: &Env) -> BackerRecord {
    BackerRecord {
        amount: 0,
        pending_amount: 0,
        pending_matures_at: 0,
        withdraw_amount: 0,
        withdraw_ready_at: 0,
        yield_index_at: storage::get_backer_yield_index(env),
        yield_owed: 0,
    }
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
}

/// Moves matured pending money into the counted balance. Returns the amount
/// moved (0 if nothing pending or not yet mature). Caller persists `record`.
/// r3: settles yield first, so newly counted money earns only from now.
fn settle_maturity(env: &Env, record: &mut BackerRecord) -> i128 {
    accrue_yield(env, record);
    if record.pending_amount <= 0 || env.ledger().timestamp() < record.pending_matures_at {
        return 0;
    }
    let moved = record.pending_amount;
    record.amount += moved;
    record.pending_amount = 0;
    record.pending_matures_at = 0;
    storage::set_total_backed_pending(env, storage::get_total_backed_pending(env) - moved);
    storage::set_total_backed(env, storage::get_total_backed(env) + moved);
    moved
}

/// Deposit. Counts toward capacity only after `BACKER_MATURITY_SECONDS`.
/// A top-up while earlier money is still maturing restarts the clock for the
/// whole pending amount (never shortens anyone's wait).
pub fn back(env: &Env, backer: &Address, amount: i128) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;
    backer.require_auth();
    if amount <= 0 {
        return Err(PoolError::AmountNotPositive);
    }

    let mut record = storage::get_backer(env, backer).unwrap_or_else(|| empty_record(env));
    let matured = settle_maturity(env, &mut record);

    let matures_at = env.ledger().timestamp() + BACKER_MATURITY_SECONDS;
    record.pending_amount += amount;
    record.pending_matures_at = matures_at;
    storage::set_backer(env, backer, &record);
    storage::set_total_backed_pending(env, storage::get_total_backed_pending(env) + amount);
    // r3: from the first backing on, vault/treasury/deploy-ceiling changes wait 7 days.
    storage::set_ever_funded(env);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    Backed { backer: backer.clone(), amount, matures_at }.publish(env);

    // Interaction last (CEI): backer to contract.
    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(backer, env.current_contract_address(), &amount);
    // v1: put idle cash above the buffer into the vault. Never fails the deposit.
    crate::vault::push_idle(env);
    Ok(())
}

/// Permissionless: once mature, a deposit starts counting toward capacity.
/// Until someone calls this, capacity is undercounted (the safe direction).
pub fn mature_backing(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.pending_amount <= 0 {
        return Err(PoolError::NoPendingBacking);
    }
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

/// Starts the notice. Only matured money can be requested; it keeps counting
/// toward capacity until the withdrawal completes (rule 2). Works while paused.
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

    let notice = crate::settings::get(env, crate::settings::SettingKey::BackerNoticeSeconds) as u64;
    let ready_at = env.ledger().timestamp() + notice;
    record.withdraw_amount = amount;
    record.withdraw_ready_at = ready_at;
    storage::set_backer(env, backer, &record);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    BackerWithdrawalRequested { backer: backer.clone(), amount, ready_at }.publish(env);
    Ok(ready_at)
}

/// Backer withdraws their own request. Works while paused.
pub fn cancel_withdrawal(env: &Env, backer: &Address) -> Result<(), PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    if record.withdraw_amount <= 0 {
        return Err(PoolError::NoBackerWithdrawal);
    }
    let amount = record.withdraw_amount;
    record.withdraw_amount = 0;
    record.withdraw_ready_at = 0;
    storage::set_backer(env, backer, &record);
    storage::bump_instance_ttl(env);
    BackerWithdrawalCancelled { backer: backer.clone(), amount }.publish(env);
    Ok(())
}

/// Pays a requested withdrawal once the notice has passed, if the capital is
/// free (rule 3) and liquid. On failure nothing changes and the request stays
/// open; the backer retries later. Pays only the backer's own address
/// (rule 1). Works while paused.
pub fn complete_withdrawal(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    let amount = record.withdraw_amount;
    if amount <= 0 {
        return Err(PoolError::NoBackerWithdrawal);
    }
    if env.ledger().timestamp() < record.withdraw_ready_at {
        return Err(PoolError::BackerNoticeNotPassed);
    }
    // Rule 3: open claims keep their backing.
    if storage::get_total_allocated(env) > storage::get_capacity(env) - amount {
        return Err(PoolError::BackerCapitalNotFree);
    }
    crate::vault::harvest(env);
    crate::vault::pull_for_payment(env, amount)?;

    // Effects before interaction (CEI).
    let matured = settle_maturity(env, &mut record);
    // r3: unpaid yield goes out with the principal (set-aside cash).
    let paid_yield = crate::vault::take_backer_yield(env, record.yield_owed);
    record.yield_owed -= paid_yield;
    record.amount -= amount;
    record.withdraw_amount = 0;
    record.withdraw_ready_at = 0;
    storage::set_backer(env, backer, &record);
    storage::set_total_backed(env, storage::get_total_backed(env) - amount);
    storage::bump_instance_ttl(env);

    if matured > 0 {
        BackingMatured { backer: backer.clone(), amount: matured }.publish(env);
    }
    BackerWithdrawn { backer: backer.clone(), amount }.publish(env);
    if paid_yield > 0 {
        crate::vault::YieldPaid { owner: backer.clone(), to: backer.clone(), amount: paid_yield }.publish(env);
    }

    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(&env.current_contract_address(), backer, &(amount + paid_yield));
    Ok(amount)
}

/// r3 (2026-09-29): take backer yield any time, no notice, no free-capital
/// check, principal untouched. Paid only to the backer's own address
/// (rule 1). Works while paused (harvest skipped then). Also matures any
/// pending money that is due, so it starts earning.
pub fn claim_yield(env: &Env, backer: &Address) -> Result<i128, PoolError> {
    backer.require_auth();
    let mut record = storage::get_backer(env, backer).ok_or(PoolError::NoBacker)?;
    crate::vault::harvest(env);
    let matured = settle_maturity(env, &mut record);
    let paid = crate::vault::take_backer_yield(env, record.yield_owed);
    if paid <= 0 {
        return Err(PoolError::NoYieldOwed);
    }
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
pub fn yield_owed_view(env: &Env, backer: &Address) -> i128 {
    match storage::get_backer(env, backer) {
        Some(mut record) => {
            accrue_yield(env, &mut record);
            record.yield_owed
        }
        None => 0,
    }
}
