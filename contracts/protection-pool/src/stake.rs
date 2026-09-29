//! Stake + withdraw principal. Ported from `SAFUPoolV8.sol`'s `stakeETH`/
//! `withdraw`, minus the Lido wstETH wrap/unwrap (excluded, Tranche 1 has
//! no yield venue). Verified against source 2026-07-14 (V8's actual
//! function is `stakeETH`, not `stake`, one staker = one active
//! StakeRecord, a second stake requires withdrawing the first: V8 line
//! `require(stakes[msg.sender].amount == 0, "already staked")`).
//!
//! Stake bounds corrected 2026-07-14 (user): as basis points of
//! the configurable pool cap (types.rs MIN_STAKE_BPS/MAX_STAKE_BPS), not
//! fixed amounts: portable across future EVM/Solana/BNB relaunches with
//! different pool sizes. Tranche 1 deploy default: 600,000 XLM pool cap
//! (see README.md "Deploy-time arguments"), approximating V8's 60 ETH.
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

use soroban_sdk::{contractevent, token::TokenClient, xdr::ToXdr, Address, Env};

use crate::error::PoolError;
use crate::storage;
use crate::types::{
    StakeRecord, LEDGERS_PER_DAY, STAKE_BPS_DENOMINATOR,
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

/// V8 `_computePoints`: tenure-banded (100/120/150/200 pts/day at
/// 0-90/91-180/181-365/365+ days) × stake/max_stake. V8 divides by the
/// FIXED STAKE_MAX; since Tranche 1 made stake bounds a share of the pool (bps of
/// pool cap), this divides by the CURRENT max_stake(pool_cap) instead,
/// flagged in the KB as a linkage decision, not silently assumed. Returns
/// 0 if the stake is empty or already withdrawn (matches V8 exactly).
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

    let now = env.ledger().sequence();
    let d = (now.saturating_sub(record.staked_at_ledger) / LEDGERS_PER_DAY) as i128;
    let d1 = d.min(90);
    let d2 = if d > 90 { d.min(180) - 90 } else { 0 };
    let d3 = if d > 180 { d.min(365) - 180 } else { 0 };
    let d4 = if d > 365 { d - 365 } else { 0 };
    let base = d1 * 100 + d2 * 120 + d3 * 150 + d4 * 200;

    let denom = max_stake(env);
    if denom <= 0 {
        return 0;
    }
    base * record.amount / denom
}

/// XLM native asset SAC address, set once at `initialize` (network-specific,
/// differs testnet/mainnet: never hardcoded in contract logic).
fn asset_token_address(env: &Env) -> Address {
    storage::get_asset_token(env)
}

/// Bounds: `pool_cap × BPS / 10_000`. Recomputed live on every
/// call, so a pool-cap change via `set_pool_cap` takes effect immediately
/// with no separate re-anchoring step.
fn min_stake(env: &Env) -> i128 {
    storage::get_pool_cap(env) * crate::settings::get(env, crate::settings::SettingKey::MinStakeBps) / STAKE_BPS_DENOMINATOR
}

fn max_stake(env: &Env) -> i128 {
    storage::get_pool_cap(env) * crate::settings::get(env, crate::settings::SettingKey::MaxStakeBps) / STAKE_BPS_DENOMINATOR
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
    if amount < min_stake(env) || amount > max_stake(env) {
        return Err(PoolError::StakeOutOfRange);
    }

    // V8: require(stakes[msg.sender].amount == 0, "already staked")
    // One active StakeRecord per address, must withdraw before re-staking.
    if let Some(existing) = storage::get_stake(env, staker) {
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
    // V8: totalStaked + msg.value <= MAX_POOL_ETH && <= maxPoolSize
    if total_staked + amount > pool_cap {
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
    if beneficiary == &storage::get_oracle(env) {
        return Err(PoolError::BeneficiaryIsOracle);
    }
    if beneficiary == &storage::get_admin(env) {
        return Err(PoolError::BeneficiaryIsAdmin);
    }
    if beneficiary == &storage::get_co_signer(env) {
        return Err(PoolError::BeneficiaryIsCoSigner);
    }

    // Verified 2026-07-14 via `cargo check` against soroban-sdk 27.0.0.
    let beneficiary_hash = env.crypto().sha256(&beneficiary.to_xdr(env)).to_bytes();

    // Effects before interaction (CEI).
    let record = StakeRecord {
        beneficiary_hash,
        amount,
        // Snapshot NOW, before any future extract_yield can move the index,
        // this staker only accrues yield realised after joining.
        yield_index_at_stake: storage::get_yield_index(env),
        staked_at_ledger: env.ledger().sequence(),
        staked_at_timestamp: env.ledger().timestamp(),
        penalty_locked_until_ledger: 0,
        withdrawn: false,
        suspended: false,
        active_claim_id: None,
        reserved_claim_id: None,
        claim_approved: false,
    };
    storage::set_stake(env, staker, &record);
    storage::set_total_staked(env, total_staked + amount);
    storage::set_total_stakers(env, storage::get_total_stakers(env) + 1);
    storage::bump_instance_ttl(env);

    Staked { staker: staker.clone(), amount }.publish(env);

    // Interaction: SAC transfer, staker to contract.
    let token = TokenClient::new(env, &asset_token_address(env));
    token.transfer(staker, env.current_contract_address(), &amount);
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
    if new_beneficiary == &storage::get_oracle(env) {
        return Err(PoolError::BeneficiaryIsOracle);
    }
    if new_beneficiary == &storage::get_admin(env) {
        return Err(PoolError::BeneficiaryIsAdmin);
    }
    if new_beneficiary == &storage::get_co_signer(env) {
        return Err(PoolError::BeneficiaryIsCoSigner);
    }

    let mut record = storage::get_stake(env, staker).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 {
        return Err(PoolError::NoStake);
    }
    if record.withdrawn {
        return Err(PoolError::StakeForfeited);
    }

    record.beneficiary_hash = env.crypto().sha256(&new_beneficiary.to_xdr(env)).to_bytes();
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

    let expected_hash = env.crypto().sha256(&beneficiary.to_xdr(env)).to_bytes();
    if expected_hash != record.beneficiary_hash {
        return Err(PoolError::WrongBeneficiary);
    }

    // V8: finalPoints computed BEFORE withdrawn=true (compute_points
    // returns 0 once withdrawn). Missing from the first two build passes.
    let final_points = compute_points(env, staker);
    let amount = record.amount;

    // Principal + this staker's proportional share of yield realised since
    // they joined. Floor division: any dust rounds toward the pool, same
    // direction every other rounding decision in this contract goes.
    // `amount` (flat principal) is still what total_staked/tier_cap use
    // below and elsewhere: only the actual token TRANSFER pays the
    // yield-inflated figure.
    let current_index = storage::get_yield_index(env);
    let payout = amount * current_index / record.yield_index_at_stake;

    // D2: before D2 this transfer could not fail on funds, `total_staked`
    // WAS the contract's real balance. Once XLM can sit in the yield vault
    // the two diverge, and without this check a short balance would surface
    // as an opaque SAC host trap. Checked before any state mutation so the
    // typed error is the only outcome; principal is never at risk, it is in
    // the vault. v1: `pull_for_payment` redeems from the vault first if cash
    // is short. Checked against `payout` (principal + accrued yield), not the
    // bare principal: the yield share must also be physically liquid.
    crate::vault::pull_for_payment(env, payout)?;

    // Effects before interaction (CEI).
    record.withdrawn = true;
    record.amount = 0;
    storage::set_stake(env, staker, &record);

    if final_points > 0 {
        let banked = storage::get_points_balance(env, staker);
        storage::set_points_balance(env, staker, banked + final_points);
    }

    // total_staked tracks committed PRINCIPAL only, never the yield-inflated
    // figure: decrementing by the flat `amount` keeps it consistent with
    // both the pool_cap admission check and tier_cap's entitlement basis.
    // Clamped at 0: a pull above may have marked a vault loss down in
    // total_staked, which can leave it below this stake (last staker out).
    let total_staked = storage::get_total_staked(env);
    storage::set_total_staked(env, (total_staked - amount).max(0));
    storage::set_total_stakers(env, storage::get_total_stakers(env).saturating_sub(1));
    storage::bump_instance_ttl(env);

    Withdrawn { staker: staker.clone(), amount: payout }.publish(env);

    // Interaction: SAC transfer, contract to beneficiary.
    let token = TokenClient::new(env, &asset_token_address(env));
    token.transfer(&env.current_contract_address(), beneficiary, &payout);
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
/// DELIBERATE SCOPE BOUNDARY (2026-09-18): pays flat `record.amount` only,
/// no yield-index reconciliation. This is a panic-time safety exit, not the
/// normal path: a staker fleeing mid-pause forgoes any accrued yield in
/// exchange for the guarantee their principal comes back with no lock.
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

    let amount = record.amount;

    // D2: same reasoning as `withdraw`, but this is the path that most
    // needs it: `emergency_exit` runs WHILE PAUSED and is the staker's
    // escape hatch. It is precisely because this function must work during
    // a pause that `vault::provide_liquidity` deliberately does NOT carry a
    // `require_not_paused` guard (V8's equivalent does; porting that
    // modifier would leave this path unfundable exactly when it matters).
    // v1: the in-path pull has no pause guard either, for the same reason.
    crate::vault::pull_for_payment(env, amount)?;

    record.withdrawn = true;
    record.amount = 0;
    storage::set_stake(env, staker, &record);

    // Clamped at 0, same reason as `withdraw`.
    let total_staked = storage::get_total_staked(env);
    storage::set_total_staked(env, (total_staked - amount).max(0));
    storage::set_total_stakers(env, storage::get_total_stakers(env).saturating_sub(1));
    storage::bump_instance_ttl(env);

    EmergencyExit { staker: staker.clone(), amount }.publish(env);

    let token = TokenClient::new(env, &asset_token_address(env));
    token.transfer(&env.current_contract_address(), staker, &amount);
    Ok(())
}
