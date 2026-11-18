//! Stake + withdraw principal. Ported from `SAFUPoolV8.sol`'s `stakeETH`/
//! `withdraw`; yield comes from the pool's vault (`vault.rs`), not a
//! per-stake wrap. Verified against source 2026-07-14 (V8's actual
//! function is `stakeETH`, not `stake`, one staker = one active
//! StakeRecord, a second stake requires withdrawing the first: V8 line
//! `require(stakes[msg.sender].amount == 0, "already staked")`).
//!
//! Stake bounds corrected 2026-07-14 (user): as basis points of
//! the configurable pool cap (types.rs MIN_STAKE_BPS/MAX_STAKE_BPS), not
//! fixed amounts: portable across future EVM/Solana/BNB relaunches with
//! different pool sizes. The pool cap is a deploy argument and a setting.
//!
//! Beneficiary handling corrected 2026-07-14 (user flagged the first draft
//! didn't match V8): takes the plaintext beneficiary `Address`, not a
//! pre-computed hash: the contract hashes it itself (sha256, not
//! keccak256: no cross-chain hash-matching requirement, see
//! types.rs::StakeRecord doc) and validates identity, matching V8's
//! `stakeETH` checks exactly.
//!
//! CONVERTED 2026-07-31: `panic!("SAFU: ...")` -> `Result<_, PoolError>`,
//! same conditions/order, no behavior change, see error.rs.

use soroban_sdk::{contractevent, xdr::ToXdr, Address, BytesN, Env};

use crate::error::PoolError;
use crate::storage;
use crate::types::{
    StakeRecord, BPS_DENOMINATOR,
};

// -----------------------------------------------------------------------
// Events: soroban-sdk 27's #[contractevent] pattern (verified against
// docs.rs/developers.stellar.org 2026-07-14), not the deprecated raw
// env.events().publish() call.
// -----------------------------------------------------------------------

#[contractevent]
pub struct Staked {
    #[topic]
    pub staker: Address,
    pub amount: i128,
}

#[contractevent]
pub struct Withdrawn {
    #[topic]
    pub staker: Address,
    pub amount: i128,
}

#[contractevent]
pub struct EmergencyExit {
    #[topic]
    pub staker: Address,
    pub amount: i128,
}

/// Points this stake has earned so far: dollar-days at the staker rate
/// (`points.rs`). 0 if the stake is empty or already withdrawn (as V8).
pub fn compute_points(env: &Env, wallet: &Address) -> i128 {
    match storage::get_stake(env, wallet) {
        Some(r) => compute_points_for_record(env, &r),
        None => 0,
    }
}

/// Same formula as `compute_points`, taking a record directly rather than
/// re-fetching from storage. Used by claim.rs's forfeiture path, which
/// needs to compute points from an in-memory (not-yet-persisted) record,
/// re-fetching would rely on storage still coincidentally matching local
/// state rather than being an explicit invariant.
pub fn compute_points_for_record(env: &Env, record: &StakeRecord) -> i128 {
    if record.amount <= 0 || record.withdrawn {
        return 0;
    }
    // Dollar-days via the staker points index.
    crate::points::earned(
        record.amount,
        record.points_index_at,
        crate::points::index(env, crate::points::STAKER),
    )
}

/// The stored form of a payout address: sha256 of its XDR (the plaintext is never stored).
pub(crate) fn beneficiary_hash(env: &Env, beneficiary: &Address) -> BytesN<32> {
    env.crypto().sha256(&beneficiary.to_xdr(env)).to_bytes()
}

/// `beneficiary` is the payout address this stake committed to.
pub(crate) fn check_beneficiary_hash(
    env: &Env,
    record: &StakeRecord,
    beneficiary: &Address,
) -> Result<(), PoolError> {
    if beneficiary_hash(env, beneficiary) != record.beneficiary_hash {
        return Err(PoolError::WrongBeneficiary);
    }
    Ok(())
}

/// Closes a stake on exit (`withdraw`, `emergency_exit`): pays out the principal plus whatever
/// unpaid yield the set-aside covers (the rest stays owed, 2026-10-20), after pulling cash from the
/// vault if short. `total_staked` tracks committed principal only, so it drops by the flat amount,
/// clamped at 0: a pull may have marked a vault loss down in it, which can leave it below this
/// stake (last staker out). Returns the payout; the caller transfers it.
fn close_stake(env: &Env, staker: &Address, record: &mut StakeRecord) -> Result<i128, PoolError> {
    let amount = record.amount;
    let owed = record.yield_owed + crate::vault::staker_yield_owed(env, record);
    let paid_yield = crate::vault::take_staker_yield(env, owed);
    let payout = amount + paid_yield;
    crate::vault::pull_for_payment(env, payout)?;

    // Effects before interaction (CEI).
    record.withdrawn = true;
    record.amount = 0;
    record.yield_owed = owed - paid_yield;
    record.yield_index_at_stake = storage::get_yield_index(env);
    storage::set_stake(env, staker, record);

    let total_staked = storage::get_total_staked(env);
    storage::set_total_staked(env, (total_staked - amount).max(0));
    storage::set_total_stakers(env, storage::get_total_stakers(env).saturating_sub(1));
    storage::bump_instance_ttl(env);
    Ok(payout)
}

/// Bounds: `pool_cap × BPS / 10_000`. Recomputed live on every
/// call, so a pool-cap change via `set_pool_cap` takes effect immediately
/// with no separate re-anchoring step.
fn min_stake(env: &Env) -> i128 {
    storage::get_pool_cap(env) * crate::settings::get(env, crate::settings::SettingKey::MinStakeBps) / BPS_DENOMINATOR
}

fn max_stake(env: &Env) -> i128 {
    storage::get_pool_cap(env) * crate::settings::get(env, crate::settings::SettingKey::MaxStakeBps) / BPS_DENOMINATOR
}

/// Payout address rules for `stake` and `set_beneficiary`: never a role key,
/// and never the pool or its vault, where a payout would just stay put
/// (2026-10-20, as on Solana).
fn check_beneficiary(env: &Env, beneficiary: &Address) -> Result<(), PoolError> {
    if beneficiary == &storage::get_oracle(env) {
        return Err(PoolError::BeneficiaryIsOracle);
    }
    if beneficiary == &storage::get_admin(env) {
        return Err(PoolError::BeneficiaryIsAdmin);
    }
    if beneficiary == &storage::get_co_signer(env) {
        return Err(PoolError::BeneficiaryIsCoSigner);
    }
    if beneficiary == &env.current_contract_address() || Some(beneficiary.clone()) == storage::get_vault(env) {
        return Err(PoolError::BeneficiaryIsPool);
    }
    Ok(())
}

pub fn stake(
    env: &Env,
    staker: &Address,
    amount: i128,
    beneficiary: &Address,
) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;
    staker.require_auth();

    if amount <= 0 {
        return Err(PoolError::StakeNotPositive);
    }
    // V8: require(msg.value >= STAKE_MIN && msg.value <= STAKE_MAX, "stake out of range")
    // Here the bounds are basis points of pool_cap instead of fixed amounts.
    // Above the max is no longer refused.
    // The max is staked and the rest becomes backing (never forfeited by a
    // claim), all in one transfer.
    if amount < min_stake(env) {
        return Err(PoolError::StakeOutOfRange);
    }
    let staked = amount.min(max_stake(env));
    let overflow = amount - staked;

    // V8: require(stakes[msg.sender].amount == 0, "already staked")
    // One active StakeRecord per address, must withdraw before re-staking.
    let mut carried_yield = 0;
    if let Some(existing) = storage::get_stake(env, staker) {
        carried_yield = existing.yield_owed;
        if existing.amount > 0 && !existing.withdrawn {
            return Err(PoolError::AlreadyStaked);
        }
        // Pre-audit hardening (rule set 2026-09-23): once a claim is
        // approved, this address never stakes again. A new record would
        // overwrite the forfeited one while its claim is still paying out.
        if existing.claim_approved {
            return Err(PoolError::AddressHasApprovedClaim);
        }
    }

    let total_staked = storage::get_total_staked(env);
    let pool_cap = storage::get_pool_cap(env);
    // V8: totalStaked + msg.value <= MAX_POOL_ETH && <= maxPoolSize.
    // A pool full for stakes refuses the WHOLE deposit (the overflow
    // never turns a refused stake into backing).
    if total_staked + staked > pool_cap {
        return Err(PoolError::PoolCapExceeded);
    }

    // V8 identity checks: beneficiary cannot be oracle, admin, or
    // coSigner. (V8 also checks beneficiary != address(0); Soroban's
    // Address has no equivalent zero sentinel, so that specific check is
    // dropped, not silently reinterpreted as something else.)
    //
    // beneficiary == staker is ALLOWED as of 2026-09-19 (design decision,
    // this fork only), the multichain covered-wallet model decouples the
    // staking wallet from the protected wallets, so a fresh/disposable
    // staking address never needs a separate payout address. Off-chain
    // registry enforces the replacement invariant: a covered wallet can
    // never equal the staker, and beneficiary can never equal a covered
    // wallet: see backend/registry.py.
    check_beneficiary(env, beneficiary)?;

    let beneficiary_hash = beneficiary_hash(env, beneficiary);

    // r3 (code review B2, 2026-09-29): recognise growth earned so far before
    // this stake counts, so it goes to the stakers who earned it. Best effort.
    crate::vault::harvest(env);
    crate::points::checkpoint(env, crate::points::STAKER);

    // Effects before interaction (CEI).
    let record = StakeRecord {
        beneficiary_hash,
        amount: staked,
        // Snapshot NOW (after the harvest above): this staker only accrues
        // yield realised after joining.
        yield_index_at_stake: storage::get_yield_index(env),
        staked_at_ledger: env.ledger().sequence(),
        staked_at_timestamp: env.ledger().timestamp(),
        penalty_locked_until_ledger: 0,
        withdrawn: false,
        suspended: false,
        suspended_until: 0,
        tier_a_ratio: crate::settings::get(env, crate::settings::SettingKey::TierARatio),
        tier_b_ratio: crate::settings::get(env, crate::settings::SettingKey::TierBRatio),
        tier_c_ratio: crate::settings::get(env, crate::settings::SettingKey::TierCRatio),
        time_gate_ledgers: crate::settings::get(env, crate::settings::SettingKey::TimeGateLedgers)
            as u32,
        claim_window_secs: crate::settings::get(env, crate::settings::SettingKey::ClaimWindowSecs)
            as u64,
        active_claim_id: None,
        reserved_claim_id: None,
        claim_approved: false,
        points_index_at: crate::points::index(env, crate::points::STAKER),
        yield_owed: carried_yield,
    };
    storage::set_stake(env, staker, &record);
    storage::set_total_staked(env, total_staked + staked);
    storage::set_total_stakers(env, storage::get_total_stakers(env) + 1);
    // r3: from the first stake on, vault/treasury/deploy-ceiling changes wait 7 days.
    storage::set_ever_funded(env);
    storage::bump_instance_ttl(env);

    Staked { staker: staker.clone(), amount: staked }.publish(env);
    if overflow > 0 {
        // Same rules as `back()`; the harvest above already ran.
        crate::backer::add_backing(env, staker, overflow)?;
    }

    // Interaction: one SAC transfer for both parts, staker to contract.
    crate::vault::receive(env, staker, amount);
    // v1: put idle cash above the buffer into the vault. Never fails the stake.
    crate::vault::push_idle(env);
    Ok(())
}

/// V8 `setBeneficiary`: update the payout-receiving address before
/// forfeiture. Locked once withdrawn. Missing from the first two build
/// passes: found on full source read.
pub fn set_beneficiary(
    env: &Env,
    staker: &Address,
    new_beneficiary: &Address,
) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;
    staker.require_auth();

    // beneficiary == staker ALLOWED here too, same 2026-09-19 decision as stake().
    check_beneficiary(env, new_beneficiary)?;

    let mut record = storage::get_stake(env, staker).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 {
        return Err(PoolError::NoStake);
    }
    if record.withdrawn {
        return Err(PoolError::StakeForfeited);
    }

    record.beneficiary_hash = beneficiary_hash(env, new_beneficiary);
    storage::set_stake(env, staker, &record);
    Ok(())
}

pub fn withdraw(env: &Env, staker: &Address, beneficiary: &Address) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;
    staker.require_auth();

    let mut record = storage::get_stake(env, staker).ok_or(PoolError::NoStake)?;

    if record.amount <= 0 {
        return Err(PoolError::NoStake);
    }
    if record.withdrawn {
        return Err(PoolError::AlreadyWithdrawn);
    }
    if record.active_claim_id.is_some() {
        return Err(PoolError::ClaimActive);
    }
    // Pre-audit hardening (H1): a queued claim holds no reservation, but it
    // is still a claim on THIS stake. Leaving now would let it be released
    // and paid later with nothing left to forfeit.
    if record.reserved_claim_id.is_some() {
        return Err(PoolError::ClaimQueuedForStake);
    }
    let now = env.ledger().sequence();
    if now < record.penalty_locked_until_ledger {
        return Err(PoolError::PenaltyLockActive);
    }

    check_beneficiary_hash(env, &record, beneficiary)?;

    // V8: finalPoints computed BEFORE withdrawn=true (compute_points
    // returns 0 once withdrawn). Missing from the first two build passes.
    let final_points = compute_points(env, staker);
    // Growth so far is credited before this staker's yield is read.
    crate::vault::harvest(env);
    let payout = close_stake(env, staker, &mut record)?;
    if final_points > 0 {
        let banked = storage::get_points_balance(env, staker);
        storage::set_points_balance(env, staker, banked + final_points);
    }

    Withdrawn { staker: staker.clone(), amount: payout }.publish(env);
    crate::vault::pay(env, beneficiary, payout);
    Ok(())
}

/// V8 `emergencyExit`: staker self-rescue during pause, no time lock.
/// V8's version returns wstETH directly (skips Lido/Curve unwind); since
/// Tranche 1 holds native XLM with no yield-deployment layer, there's
/// nothing special to unwind: this is functionally identical to
/// withdraw() except it works WHILE paused (withdraw() is blocked then)
/// and skips the beneficiary-hash check (V8's emergencyExit sends
/// directly to msg.sender, not a separate beneficiary, same here).
///
/// r3 (2026-09-29): also pays the staker's unpaid yield from the set-aside
/// (design rule: yield is never lost). No harvest here: this runs while
/// paused, and only already-credited yield is paid. Replaces the 2026-09-18
/// rule that an exit forgoes yield.
pub fn emergency_exit(env: &Env, staker: &Address) -> Result<(), PoolError> {
    staker.require_auth();

    let mut record = storage::get_stake(env, staker).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 || record.withdrawn {
        return Err(PoolError::NoActiveStake);
    }
    if record.active_claim_id.is_some() {
        return Err(PoolError::ClaimActive);
    }
    // Same guard as `withdraw` (H1).
    if record.reserved_claim_id.is_some() {
        return Err(PoolError::ClaimQueuedForStake);
    }
    // 2026-10-20: the 365-day lock after a cancelled claim holds here too.
    if env.ledger().sequence() < record.penalty_locked_until_ledger {
        return Err(PoolError::PenaltyLockActive);
    }

    // This path must work during a pause (it is the escape hatch), so neither
    // the vault pull nor `vault::provide_liquidity` carries a pause guard.
    let payout = close_stake(env, staker, &mut record)?;

    EmergencyExit { staker: staker.clone(), amount: payout }.publish(env);
    crate::vault::pay(env, staker, payout);
    Ok(())
}

/// r3 (2026-09-29): take yield without unstaking. Design rule: any time, no
/// wait, no pool-health check; the stake, its coverage and its 90-day clock
/// are untouched (coverage is principal only). Works while paused (the
/// harvest is skipped then, already-credited yield is still paid).
///
/// Paid to the beneficiary, hash-checked exactly as in `withdraw`. A
/// claim never takes the claimant's yield, so this works with a claim
/// open or queued, and after one is approved (the yield earned up to the
/// approval was settled into `yield_owed` then).
pub fn claim_yield(env: &Env, staker: &Address, beneficiary: &Address) -> Result<i128, PoolError> {
    staker.require_auth();
    let mut record = storage::get_stake(env, staker).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 && record.yield_owed <= 0 {
        return Err(PoolError::NoActiveStake);
    }
    check_beneficiary_hash(env, &record, beneficiary)?;

    crate::vault::harvest(env);
    let owed = record.yield_owed + crate::vault::staker_yield_owed(env, &record);
    let paid = crate::vault::take_staker_yield(env, owed);
    if paid <= 0 {
        return Err(PoolError::NoYieldOwed);
    }
    crate::vault::pull_for_payment(env, paid)?;

    // Effects before interaction (CEI). Any part the set-aside can't cover
    // yet stays owed (2026-10-20).
    record.yield_owed = owed - paid;
    record.yield_index_at_stake = storage::get_yield_index(env);
    storage::set_stake(env, staker, &record);
    storage::bump_instance_ttl(env);
    crate::vault::YieldPaid { owner: staker.clone(), to: beneficiary.clone(), amount: paid }.publish(env);

    crate::vault::pay(env, beneficiary, paid);
    Ok(paid)
}
