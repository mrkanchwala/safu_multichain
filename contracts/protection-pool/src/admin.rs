//! Admin: one-time init + oracle/coSigner rotation. Ported from V8's
//! constructor + `setOracle`/`setCoSigner`. The 2-of-2 override
//! request/approval flow lives in claim.rs instead (Task 3), it operates
//! on Claim/OverrideRequest records and is conceptually part of the
//! claims subsystem, not admin config.
//!
//! CONVERTED 2026-07-31: `panic!("SAFU: ...")` -> `Result<_, PoolError>`.
//! Every function that used to trap now returns `Err(PoolError::...)` on
//! the same conditions, in the same order, with no behavior change beyond
//! the caller now getting a typed error code instead of an opaque panic
//! message. See `error.rs` for the enum.

use soroban_sdk::{contractevent, Address, BytesN, Env};

use crate::error::PoolError;
use crate::storage;
use crate::settings::{get as setting, SettingKey};
use crate::types::{ClaimStatus, StakeRecord, STORAGE_VERSION, SUSPEND_MAX_SECONDS};

// Pre-audit hardening (2026-09-23, M2): admin actions emit events, so stakers,
// monitors and the keeper see every one (T3's Repudiate.2 finding).

/// A pause ends on its own after this long (default of `PauseMaxSecs`). So a
/// pause can never freeze staker money for good, even if keys are lost.
/// 2026-10-16: it can't be renewed back to back either; the next pause
/// waits `PauseGapSecs` after this one ends.
pub const PAUSE_MAX_SECONDS: u64 = 30 * crate::types::SECONDS_PER_DAY;

#[contractevent]
pub struct Paused {
    pub by: Address,
    pub until: u64,
}

#[contractevent]
pub struct Unpaused {
    pub by: Address,
}

#[contractevent]
pub struct PoolCapChanged {
    pub old_cap: i128,
    pub new_cap: i128,
}

#[contractevent]
pub struct StakeSuspended {
    #[topic]
    pub wallet: Address,
}

#[contractevent]
pub struct SuspensionRenewed {
    #[topic]
    pub wallet: Address,
    pub approver: Address,
    pub until: u64,
}

#[contractevent]
pub struct StakeUnsuspended {
    #[topic]
    pub wallet: Address,
}

/// Implementation behind the contract's `__constructor` (`lib.rs`). Kept
/// named `initialize` because it is an internal helper, not the ABI surface,
/// the ABI name is what had to change (7a audit, Finding 3).
///
/// Reinitialization guard (vuln checklist V6), `has()` check before any
/// state is written, including before returning early. **Unreachable through
/// the ABI as of 2026-08-17**: `__constructor` runs only at contract creation
/// and cannot be invoked again, so nothing can call this twice on-chain. Kept
/// deliberately rather than deleted: it is the invariant this function
/// depends on, it costs one storage read, and it keeps the function correct if
/// it is ever called from anywhere else. Same reasoning the override flow's
/// degenerate admin==co_signer branch is kept: provably unreachable, still
/// documented. `test/admin_tests.rs` asserts it directly.
/// The Tranche 1 deploy-time `pool_cap` value (600,000 XLM, approximating
/// V8's 60 ETH cap) is documented in README.md, not as a dead constant
/// here: `pool_cap` is a plain `initialize` argument, never hardcoded
/// into contract logic, and stays admin-adjustable afterward via
/// `set_pool_cap` (mirrors V8's mutable `maxPoolSize`). A constant that's
/// never referenced by any code path belongs in deploy docs, not source.
///
/// CHANGED for T2/D1: takes `oracle_pubkey` as well as `oracle`. The oracle
/// has TWO identities in this contract, an `Address` (policy: auth, rate
/// limit, beneficiary guard, admin invariants) and an Ed25519 pubkey
/// (attestation: signature verification). Requiring both here is the
/// "startup invariant that both are set" from the D1 eng review: it removes
/// the window in which a deployment has a working oracle Address but no
/// attestation key. `submit_claim` still fails closed on a missing key
/// (`OraclePubKeyNotSet`) as defence in depth, but with this argument that
/// state is unreachable on a fresh deploy.
#[allow(clippy::too_many_arguments)]
pub fn initialize(
    env: &Env,
    admin: &Address,
    oracle: &Address,
    oracle_pubkey: &BytesN<32>,
    co_signer: &Address,
    guardian: &Address,
    xlm_token: &Address,
    pool_cap: i128,
) -> Result<(), PoolError> {
    if env.storage().instance().has(&crate::storage::DataKey::Admin) {
        return Err(PoolError::AlreadyInitialized);
    }
    // V8 (audit checklist item): oracle != coSigner enforced at
    // construction and at every setter. V8 constructor also requires
    // coSigner != owner (msg.sender), missing from the first build pass,
    // found on full source read (2026-07-14).
    if oracle == co_signer {
        return Err(PoolError::OracleEqualsCoSigner);
    }
    if co_signer == admin {
        return Err(PoolError::CoSignerEqualsAdmin);
    }
    // Pre-audit hardening: the guardian is the third governance role.
    crate::governance::check_roles_distinct(admin, co_signer, guardian, oracle)?;
    if pool_cap <= 0 {
        return Err(PoolError::PoolCapNotPositive);
    }

    admin.require_auth();

    storage::set_admin(env, admin);
    storage::set_oracle(env, oracle);
    storage::set_oracle_pubkey(env, oracle_pubkey);
    storage::set_co_signer(env, co_signer);
    storage::set_guardian(env, guardian);
    storage::set_asset_token(env, xlm_token);
    storage::set_pool_cap(env, pool_cap);
    storage::set_total_staked(env, 0);
    storage::set_total_allocated(env, 0);
    storage::set_total_stakers(env, 0);
    storage::set_paused_until(env, 0);
    storage::set_storage_version(env, STORAGE_VERSION);
    storage::bump_instance_ttl(env);
    Ok(())
}

/// V8: pause()/unpause(), Pausable circuit breaker, admin-only. Blocks
/// stake/withdraw/submit_claim/claim_stream/unlock_pending_claim/
/// approve_override while paused; emergency_exit (stake.rs) remains the
/// pause-time exit path. Missing entirely from the first two build
/// passes: found on full source read.
///
/// 2026-10-16: a pause can't follow the last one until
/// `PauseGapSecs` after it ended, so back-to-back pauses can't hold claims
/// forever. Paused time is also taken off every claim clock.
pub fn pause(env: &Env) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();
    if storage::is_paused(env) {
        return Err(PoolError::AlreadyPaused);
    }
    let now = env.ledger().timestamp();
    let last_start = storage::get_pause_start(env);
    if last_start > 0 {
        // The last pause is over: its end is `paused_until` (set to the
        // unpause time by `unpause`).
        let last_end = storage::get_paused_until(env);
        if now < last_end + setting(env, SettingKey::PauseGapSecs) as u64 {
            return Err(PoolError::PauseGapNotMet);
        }
        let settled = storage::get_paused_secs_settled(env);
        storage::set_paused_secs_settled(env, settled + last_end.saturating_sub(last_start));
    }
    let until = now + setting(env, SettingKey::PauseMaxSecs) as u64;
    storage::set_pause_start(env, now);
    storage::set_paused_until(env, until);
    storage::bump_instance_ttl(env);
    Paused { by: admin, until }.publish(env);
    Ok(())
}

/// Ends the pause now. The end time is kept (not zeroed) so the pause gap
/// and the paused-time count both see it.
pub fn unpause(env: &Env) {
    let admin = storage::get_admin(env);
    admin.require_auth();
    if storage::is_paused(env) {
        storage::set_paused_until(env, env.ledger().timestamp());
    }
    Unpaused { by: admin }.publish(env);
}

/// V8: suspendStake/unsuspendStake: blocks payout eligibility, does NOT
/// block principal withdrawal (suspended stakers can still exit).
///
/// CHANGED 2026-07-22 (eng review, bug 5 / suspend upgrade): previously
/// only blocked a brand-new `submit_claim`, a claim already filed
/// (PendingTime/AwaitingApproval) still activated on schedule, and an
/// already-Active claim kept streaming, completely unaffected by a
/// suspension applied after the fact. `claim.rs`'s `approve_claim` and
/// `claim_stream` now both check `suspended` directly, so this same
/// `suspended = true` flag now also freezes an in-progress claim.
///
/// CORRECTED 2026-07-22 (adversarial /audit-chain re-review, same day):
/// the original fix above was still dead on arrival, this function's
/// pre-existing `if record.withdrawn { panic! }` guard meant admin could
/// NEVER reach `suspend_stake` on a stake that had already forfeited via
/// `approve_claim`/an override, which is exactly the Active/streaming
/// case the upgrade was meant to reach. `withdrawn` serves double duty in
/// this contract (a genuinely-done voluntary exit, OR forfeiture from an
/// in-progress claim), the guard needs to allow the second case and
/// still block the first. `active_claim_id` is what actually
/// distinguishes them: `Some` means a live claim still exists (still
/// suspendable), `None` means the wallet is genuinely finished (nothing
/// left to freeze).
///
/// 2026-10-16: the admin alone suspends for at most
/// `SUSPEND_MAX_SECONDS`, then it lapses. Suspending again while one is in
/// force, or within that same length after it ends, needs a second role
/// (`renew_suspension`). So one key can't freeze a claimant for good.
pub fn suspend_stake(env: &Env, wallet: &Address) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    let mut record = suspendable(env, wallet)?;
    let now = env.ledger().timestamp();
    if record.suspended_until > 0 && now < record.suspended_until + SUSPEND_MAX_SECONDS {
        return Err(PoolError::SuspendNeedsSecondRole);
    }
    record.suspended = true;
    record.suspended_until = now + SUSPEND_MAX_SECONDS;
    storage::set_stake(env, wallet, &record);
    StakeSuspended { wallet: wallet.clone() }.publish(env);
    Ok(())
}

/// 2026-10-16: admin plus a second role (co-signer or guardian) start or
/// extend a suspension: it runs `SUSPEND_MAX_SECONDS` from now.
pub fn renew_suspension(env: &Env, wallet: &Address, approver: &Address) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();
    if approver != &storage::get_co_signer(env) && approver != &storage::get_guardian(env) {
        return Err(PoolError::ApproverNotSecondRole);
    }
    approver.require_auth();

    let mut record = suspendable(env, wallet)?;
    let until = env.ledger().timestamp() + SUSPEND_MAX_SECONDS;
    record.suspended = true;
    record.suspended_until = until;
    storage::set_stake(env, wallet, &record);
    SuspensionRenewed { wallet: wallet.clone(), approver: approver.clone(), until }.publish(env);
    Ok(())
}

fn suspendable(env: &Env, wallet: &Address) -> Result<StakeRecord, PoolError> {
    let record: StakeRecord = storage::get_stake(env, wallet).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 {
        return Err(PoolError::NoStake);
    }
    if record.withdrawn && record.active_claim_id.is_none() {
        return Err(PoolError::AlreadyWithdrawn);
    }
    Ok(record)
}

/// 2026-10-16: a suspension is in force only until `suspended_until`.
pub fn is_suspended(env: &Env, record: &StakeRecord) -> bool {
    record.suspended && env.ledger().timestamp() < record.suspended_until
}

/// CHANGED 2026-07-22 (eng review blocker #1): takes an optional
/// `claim_id` so admin can reset whichever new deadline clock (Rule A's
/// `approve_deadline_ledger` or Rule B's `last_collected_ledger`) was
/// running against this wallet's claim while suspended, otherwise a
/// staker could lose their entitlement to Rule A/B expiry purely because
/// admin froze them, through no fault of their own. Resets the relevant
/// clock to "now" (extends the full window fresh) rather than trying to
/// credit back exact suspended duration, simpler, and errs in the
/// staker's favor. A no-op on the clock if `claim_id` is `None`, the
/// claim doesn't exist, or it's in neither AwaitingApproval nor Active
/// (nothing to reset), admin can still unsuspend freely in those cases.
///
/// CORRECTED 2026-07-22 (adversarial /audit-chain re-review, same day):
/// the original version never checked `claim.wallet == wallet`, an
/// admin passing a mismatched claim_id would reset an unrelated wallet's
/// deadline instead of (or as well as) the one actually being
/// unsuspended. Only exploitable by the admin key itself (already fully
/// trusted throughout this contract: cancel_claim, suspend_stake, etc.
/// all assume a non-malicious admin), so LOW severity, but cheap and
/// worth closing: skip the reset silently on a mismatch rather than
/// acting on the wrong wallet's claim.
pub fn unsuspend_stake(
    env: &Env,
    wallet: &Address,
    claim_id: Option<BytesN<32>>,
) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    let mut record: StakeRecord = storage::get_stake(env, wallet).ok_or(PoolError::NoStake)?;
    if record.amount <= 0 {
        return Err(PoolError::NoStake);
    }
    let now = env.ledger().timestamp();
    if is_suspended(env, &record) {
        // Ends now; claim clocks restart from here (they never start
        // before `suspended_until`).
        record.suspended_until = now;
    }
    record.suspended = false;
    storage::set_stake(env, wallet, &record);
    StakeUnsuspended { wallet: wallet.clone() }.publish(env);

    if let Some(id) = claim_id {
        if let Some(mut claim) = storage::get_claim(env, &id) {
            if &claim.wallet != wallet {
                return Ok(());
            }
            if matches!(claim.status, ClaimStatus::AwaitingApproval | ClaimStatus::Active) {
                crate::claim::start_clock(env, &mut claim, now);
                storage::set_claim(env, &id, &claim);
            }
        }
    }
    Ok(())
}

/// Mirrors V8's `setPoolSize`: admin-adjustable operational cap.
pub fn set_pool_cap(env: &Env, new_cap: i128) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    if new_cap <= 0 {
        return Err(PoolError::PoolCapNotPositive);
    }
    // Don't allow shrinking below what's already staked, would make the
    // pool immediately "full" or leave min/max stake bounds inconsistent
    // with live state.
    if new_cap < storage::get_total_staked(env) {
        return Err(PoolError::PoolCapBelowTotalStaked);
    }
    // v1 (2026-09-22): once anyone has staked, this is a brake only. Raising
    // the cap then needs the 7-day settings timelock (SettingKey::PoolCap),
    // so stakers see a bigger pool coming. Before the first stake the admin
    // sets it freely (deployment setup).
    if new_cap > storage::get_pool_cap(env) && storage::get_total_staked(env) > 0 {
        return Err(PoolError::PoolCapRaiseNeedsTimelock);
    }
    let old_cap = storage::get_pool_cap(env);
    storage::set_pool_cap(env, new_cap);
    storage::bump_instance_ttl(env);
    PoolCapChanged { old_cap, new_cap }.publish(env);
    Ok(())
}
