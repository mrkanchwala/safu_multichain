//! Claims: submit → activate (7d cooldown, either immediately or via
//! unlock_pending_claim once the 90-day time gate is met) → claim_stream
//! (45d vesting, FCFS-per-day daily outflow cap) → completed, or
//! cancel_claim (false-positive reversal, 365-day penalty lock only when
//! the stake had already been forfeited). Also owns the 2-of-2
//! (admin + coSigner) override request/approval flow, a separate escape
//! hatch that bypasses the oracle/time-gate path entirely.
//!
//! Ported against context/knowledge/smartcontract-soroban.md §1b
//! (research-ops repo), verified line-by-line against SAFUPoolV8.sol,
//! not assumed from a mechanics summary.
//!
//! Auth model: `submit_claim` takes an explicit `caller: Address`
//! (oracle or admin), requires `caller.require_auth()`. Soroban's host
//! authenticates the exact function + argument tuple invoked, there is
//! no separate signed byte-blob to design; the call's own arguments ARE
//! the oracle's attested verdict. This is what closed the "payload shape"
//! open question the original stub left flagged.
//!
//! `cancel_pending_override` was initially implemented as an unverified
//! approximation, then corrected 2026-07-14 by reading V8's actual
//! function directly (`onlyOwner`, not admin-or-coSigner as first
//! guessed; deletes rather than resets). That same read also caught a
//! deeper mismatch: V8 never zeroes `StakeRecord.amount` on
//! claim-triggered forfeiture (only `withdrawn = true`), only the
//! VOLUNTARY `withdraw()` function zeroes it. This build had zeroed it
//! in both paths, which forced a wrong workaround in `execute_override`
//! (falling back to a prior claim's remembered `stake` field) and masked
//! a real accounting bug: releasing `total_allocated` for ANY
//! non-completed prior claim instead of only Active/PendingTime, double-
//! releasing when overriding an already-cancelled claim_id. All fixed
//! together: see the affected functions below for what changed and why.
//!
//! CONVERTED 2026-07-31: `panic!("SAFU: ...")` -> `Result<_, PoolError>`,
//! same conditions/order, no behavior change beyond a typed error code
//! instead of an opaque panic message, see error.rs. `tier_ratio`/`tier_cap` (internal helpers called
//! from several functions below) became fallible too, since the invalid-
//! tier check they own is externally reachable from submit_claim/
//! approve_override/execute_override's caller-supplied `tier` argument.

use soroban_sdk::{contractevent, Address, Bytes, BytesN, Env, Symbol};
use soroban_sdk::token::TokenClient;
use soroban_sdk::xdr::ToXdr;

use crate::error::PoolError;
use crate::stake;
use crate::storage;
use crate::types::{
    Claim, ClaimStatus, OverrideRequest, StakeRecord, APPROVE_WINDOW_LEDGERS, BPS_DENOMINATOR,
    CLAIM_WINDOW_SECONDS, COLLECTION_INACTIVITY_LEDGERS,
    MAX_APPROVAL_WINDOW_SECONDS, PENALTY_LOCK_LEDGERS, TIER_A_RATIO, TIER_B_RATIO, TIER_C_RATIO,
    TIER_BPS_DENOMINATOR, TIER_COVERAGE_BPS, TIME_GATE_LEDGERS,
};

/// Domain separator for the oracle approval payload, deliberately NOT
/// V8's `"SAFU_CLAIM_APPROVAL"`. The two chains use different curves
/// (secp256k1 vs Ed25519) and different key material, so cross-verification
/// is already impossible, but a distinct domain makes that a property of
/// the message rather than an accident of the crypto: a payload built for
/// one chain cannot even be *constructed* as a valid payload for the other.
/// 27 chars, within Soroban's 32-char `Symbol` limit.
const APPROVAL_DOMAIN: &str = "SAFU_CLAIM_APPROVAL_SOROBAN";

// -----------------------------------------------------------------------
// Events: #[contractevent] pattern, see stake.rs for the same note.
// -----------------------------------------------------------------------

#[contractevent]
pub struct ClaimSubmitted {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub entitlement: i128,
}

/// T3 (2026-08-24): `submit_claim` could not admit the claim right now
/// (`DailyStressCapExceeded`, `Insolvent`, or `OracleDailyClaimLimitReached`)
/// so it was queued rather than rejected, `ClaimStatus::Reserved`, publicly
/// readable via the existing `get_claim`, same as any other claim state.
#[contractevent]
pub struct ClaimQueued {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub entitlement: i128,
}

/// T3: a queued claim's blocker cleared and it was admitted for real.
#[contractevent]
pub struct ClaimQueueReleased {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
}

/// T3: a queued claim's window ran out before it ever cleared.
#[contractevent]
pub struct ClaimQueueExpired {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
}

#[contractevent]
pub struct ClaimUnlocked {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
}

/// NEW 2026-07-22 (Rule A), emitted by `approve_claim`, the staker's own
/// action that triggers points burn + forfeiture + cooldown/vesting start.
#[contractevent]
pub struct ClaimApproved {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub points_burned: i128,
}

/// NEW 2026-07-22: emitted by either `expire_pending_approval` (Rule A)
/// or `expire_stale_claim` (Rule B). `released` is whatever was returned
/// to the pool.
#[contractevent]
pub struct ClaimExpired {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub released: i128,
}

#[contractevent]
pub struct ClaimStreamed {
    #[topic]
    pub wallet: Address,
    pub amount: i128,
}

#[contractevent]
pub struct ClaimCancelled {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
}

/// T3 fix (2026-08-24): `points_burned` added, mirroring `ClaimApproved`.
/// The burn itself was always correct (`activate_claim` always ran), only
/// the event was silent about it.
#[contractevent]
pub struct OverrideExecuted {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub points_burned: i128,
}

/// Pre-audit hardening (M2): each signer's approval is public, so the other
/// signer and monitors see a pending override before it executes.
#[contractevent]
pub struct OverrideApproved {
    #[topic]
    pub wallet: Address,
    pub claim_id: BytesN<32>,
    pub approver: Address,
    pub entitlement: i128,
    pub tier: u32,
}

#[contractevent]
pub struct OverrideCancelled {
    #[topic]
    pub claim_id: BytesN<32>,
}

/// NEW T2/D1: mirrors V8's `ApprovalRevoked(approvalHash)` event (`:839`),
/// with `wallet` added as a topic so an operator can filter revocations by
/// the wallet they affect (V8's hash-only event is not filterable that way).
#[contractevent]
pub struct ApprovalRevoked {
    #[topic]
    pub wallet: Address,
    pub approval_hash: BytesN<32>,
}

// -----------------------------------------------------------------------
// Helpers: exact formulas, per KB §1b.
// -----------------------------------------------------------------------

fn tier_ratio(tier: u32) -> Result<i128, PoolError> {
    match tier {
        1 => Ok(TIER_A_RATIO),
        2 => Ok(TIER_B_RATIO),
        3 => Ok(TIER_C_RATIO),
        _ => Err(PoolError::InvalidTier),
    }
}

/// `stake × tier_ratio × TIER_COVERAGE_BPS / 10_000`, two knobs (ratio,
/// coverage-bps), not one flat multiplier. Errors on an invalid tier.
pub fn tier_cap(stake_amount: i128, tier: u32) -> Result<i128, PoolError> {
    Ok(stake_amount * tier_ratio(tier)? * TIER_COVERAGE_BPS / TIER_BPS_DENOMINATOR)
}

/// Admission-side daily cap on new-claim entitlement. Separate counter
/// from the payout-side daily outflow cap below.
pub fn stress_cap(env: &Env) -> i128 {
    // v1: sized against capacity (stakers + matured backers).
    let capacity = storage::get_capacity(env);
    if capacity == 0 {
        return 0;
    }
    let total_allocated = storage::get_total_allocated(env);
    let utilization_bps = total_allocated * BPS_DENOMINATOR / capacity;
    use crate::settings::{get as setting, SettingKey};
    let rate_bps = if utilization_bps < 2_000 {
        setting(env, SettingKey::AdmitLowBps)
    } else if utilization_bps < 5_000 {
        setting(env, SettingKey::AdmitMidBps)
    } else {
        setting(env, SettingKey::AdmitHighBps)
    };
    capacity * rate_bps / BPS_DENOMINATOR
}

/// Payout-side daily outflow cap rate, as a function of pool utilization
/// against `base` (the caller passes `max(total_staked_now,
/// claim.total_staked_snapshot)`: the anti-manipulation guarantee this
/// protects depends on that exact base, not on this function).
pub fn dynamic_outflow_bps(env: &Env, base: i128) -> i128 {
    if base == 0 {
        return 100;
    }
    let total_allocated = storage::get_total_allocated(env);
    let utilization_bps = total_allocated * BPS_DENOMINATOR / base;
    use crate::settings::{get as setting, SettingKey};
    if utilization_bps < 2_000 {
        setting(env, SettingKey::PayoutLowBps)
    } else if utilization_bps < 5_000 {
        setting(env, SettingKey::PayoutMidBps)
    } else {
        setting(env, SettingKey::PayoutHighBps)
    }
}

fn current_day(env: &Env) -> u32 {
    (env.ledger().timestamp() / crate::types::SECONDS_PER_DAY) as u32
}

/// Claim id is derived on-chain from (wallet, tx_hash), never trusted as
/// a caller-supplied value, including in the override flow.
fn compute_claim_id(env: &Env, wallet: &Address, tx_hash: &BytesN<32>) -> BytesN<32> {
    let mut buf: Bytes = wallet.to_xdr(env);
    buf.append(&Bytes::from_array(env, &tx_hash.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}

// -----------------------------------------------------------------------
// D1 (T2), oracle approval payload + Ed25519 verification.
//
// Ported from `SAFUPoolV8.sol:412-424`, which is a live-on-mainnet
// payload. This is a port, not a fresh design; the deviations below are
// forced by chain differences, not preference.
// -----------------------------------------------------------------------

/// Builds the exact byte string the oracle signs.
///
/// **Encoding: XDR per component, NOT packed concatenation.** V8 uses
/// `abi.encodePacked` (`:415-420`), which is a known EVM footgun: adjacent
/// variable-length fields are concatenated with no delimiter, so distinct
/// field tuples can serialise to identical bytes. Every component here is
/// individually XDR-serialised into a self-delimiting `ScVal` first, which
/// removes that class of ambiguity entirely while staying trivially
/// reproducible off-chain (`api/signer.py` concatenates the equivalent
/// `stellar_sdk.scval.to_*(...).to_xdr_bytes()` calls in this same order).
///
/// A single `#[contracttype]` struct serialised in one shot would also be
/// unambiguous, but it encodes as an `ScMap` whose key ordering the off-chain
/// signer would have to reproduce exactly, more cross-language risk for no
/// extra safety, since per-component XDR is already collision-free.
///
/// **Field order mirrors V8 exactly**, with two substitutions:
/// - `address(this)` -> `env.current_contract_address()`
/// - `block.chainid` -> `env.ledger().network_id()`. Read live from the
///   host, never a value baked in at `initialize` (the original plan
///   assumed no Soroban equivalent existed, it does, `ledger.rs:102`).
///   Reading it live means no deployer-supplied trust and no migration if a
///   stored value were ever set wrong.
pub fn build_approval_payload(
    env: &Env,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: i128,
    tier: u32,
    hack_timestamp: u64,
    deadline: u64,
) -> Bytes {
    let mut buf: Bytes = Symbol::new(env, APPROVAL_DOMAIN).to_xdr(env);
    buf.append(&env.current_contract_address().to_xdr(env));
    buf.append(&env.ledger().network_id().to_xdr(env));
    buf.append(&wallet.to_xdr(env));
    buf.append(&tx_hash.clone().to_xdr(env));
    buf.append(&entitlement.to_xdr(env));
    buf.append(&tier.to_xdr(env));
    buf.append(&hack_timestamp.to_xdr(env));
    buf.append(&deadline.to_xdr(env));
    buf
}

/// Revocation key: sha256 of the payload. Direct analogue of V8's
/// `revokedApprovals[keccak256(abi.encodePacked(inner))]` (`:423`), using
/// the Soroban-native hash for the same reason `compute_claim_id` and the
/// beneficiary hash already do.
fn approval_hash(env: &Env, payload: &Bytes) -> BytesN<32> {
    env.crypto().sha256(payload).to_bytes()
}

/// Verifies an oracle-signed approval.
///
/// **The check ORDER is the security property here, not an implementation
/// detail: do not reorder.** `env.crypto().ed25519_verify` returns `()`
/// and panics on failure (soroban-sdk 27.0.0, `crypto.rs:152`); there is no
/// `try_` variant and a host trap cannot be caught in-guest. Every
/// recoverable condition is therefore checked and returned as a typed
/// `PoolError` BEFORE the verify call, so the only way to reach the opaque
/// trap is a genuine cryptographic mismatch.
///
/// The trap is state-safe: per the Stellar development guidance, "an Err
/// return or panic rolls back all state changes of the invocation,
/// including nested calls' writes", and this function is called before
/// `submit_claim` writes anything. The residual cost is diagnostic only, a
/// bad signature surfaces as a generic host error rather than a named one.
/// Accepted and documented (eng review Blocker 3); unavoidable at SDK 27.
#[allow(clippy::too_many_arguments)]
fn verify_oracle_signature(
    env: &Env,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: i128,
    tier: u32,
    hack_timestamp: u64,
    deadline: u64,
    signature: &BytesN<64>,
) -> Result<(), PoolError> {
    let now_ts = env.ledger().timestamp();

    // 1. Deadline still open. V8 `:414`.
    if now_ts > deadline {
        return Err(PoolError::SignatureExpired);
    }
    // 2. Deadline within the bounded window. No V8 equivalent, V8's
    //    revocation mapping is permanent, so it never needed a deadline
    //    bound. Ours lives in temporary storage, and bounding the deadline
    //    is what makes the revocation TTL provably outlive it (types.rs).
    if deadline > now_ts + MAX_APPROVAL_WINDOW_SECONDS {
        return Err(PoolError::SignatureDeadlineTooFar);
    }
    // 3. Attestation key configured. Fail-closed and typed, rather than
    //    unwrapping into a trap indistinguishable from a bad signature.
    let pubkey = storage::get_oracle_pubkey(env).ok_or(PoolError::OraclePubKeyNotSet)?;

    let payload = build_approval_payload(
        env,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        deadline,
    );

    // 4. Not revoked. V8 `:423`.
    if storage::is_approval_revoked(env, &approval_hash(env, &payload)) {
        return Err(PoolError::ApprovalRevoked);
    }

    // 5. Only a real cryptographic mismatch can trap past this point.
    env.crypto().ed25519_verify(&pubkey, &payload, signature);
    Ok(())
}

// -----------------------------------------------------------------------
// Activation: shared by `approve_claim` (staker-gated, Rule A) and the
// override flow (admin+coSigner, bypasses Rule A entirely, two humans
// already made the decision, no separate staker approval needed). Forfeits
// the stake, banks final points, snapshots total_staked BEFORE this
// claim's own decrement (per the corrected timing note in
// types.rs::Claim::total_staked_snapshot), and sets fresh cooldown/vesting
// deadlines from "now".
//
// CHANGED 2026-07-22 (points burn-on-claim mechanism, locked 2026-07-22):
// no longer called from submit_claim/unlock_pending_claim, meeting the
// 90-day time gate now only moves a claim to AwaitingApproval; activation
// (and therefore burn) happens only via approve_claim or an override.
// Also now burns the wallet's ENTIRE lifetime points_balance (this record's
// points and every earlier cycle's), deliberate: a staker who has
// cycled through multiple stake/unstake cycles has a bigger balance, and
// burning all of it, earlier cycles included, is the intended weight on the
// claim decision. Order matters: bank this record's points into the
// balance FIRST, then zero the whole thing, never the reverse, or this
// cycle's own points would escape the burn.
// -----------------------------------------------------------------------

fn activate_claim(env: &Env, stake_record: &mut StakeRecord, claim: &mut Claim) -> i128 {
    let points = stake::compute_points_for_record(env, stake_record);
    let banked = storage::get_points_balance(env, &claim.wallet);
    let lifetime_balance = banked + points;
    storage::set_points_balance(env, &claim.wallet, 0);

    let stake_amount = stake_record.amount;
    stake_record.withdrawn = true;
    // Pre-audit hardening (H2): approved means this address never stakes again.
    stake_record.claim_approved = true;
    // Deliberately NOT zeroed here: verified against the live V8 source
    // (submitClaim's forfeiture path only sets `s.withdrawn = true`,
    // never touches `s.amount`; only the VOLUNTARY withdraw() function
    // zeroes it). `withdrawn` alone is the forfeiture gate everywhere
    // else in this contract (submit_claim, set_beneficiary, withdraw,
    // emergency_exit all check it). Keeping `amount` live lets
    // execute_override read it directly on a re-execution instead of
    // needing a fallback to a prior claim's remembered `stake` field,
    // an earlier version of this function zeroed it and had to work
    // around the consequence; fixed 2026-07-14 after reading V8 directly.

    let total_staked = storage::get_total_staked(env);
    // v1: snapshot capacity (stakers + matured backers), before this forfeiture.
    claim.total_staked_snapshot = storage::get_capacity(env);
    storage::set_total_staked(env, total_staked - stake_amount);
    storage::set_total_stakers(env, storage::get_total_stakers(env).saturating_sub(1));

    let now_ledger = env.ledger().sequence();
    claim.stake = stake_amount;
    claim.cooldown_ends_ledger = now_ledger + crate::settings::get(env, crate::settings::SettingKey::CooldownLedgers) as u32;
    claim.vesting_ends_ledger = claim.cooldown_ends_ledger + crate::settings::get(env, crate::settings::SettingKey::VestingLedgers) as u32;
    // Rule B's inactivity clock anchors at cooldown END, not at this
    // activation moment: eng review blocker #2: the mandatory 7-day
    // cooldown (during which claim_stream is blocked entirely) must never
    // count against the staker as inactivity.
    claim.last_collected_ledger = claim.cooldown_ends_ledger;
    claim.status = ClaimStatus::Active;

    lifetime_balance
}

// -----------------------------------------------------------------------
// admit_claim: shared by `submit_claim` (fresh admission) and
// `try_release_queued_claim` (T3: release of a `Reserved` claim once its
// blocker clears). Both paths must do EXACTLY the same admission
// side-effects, so this is the one place that does them, extracted
// 2026-08-24 specifically so the two callers cannot drift apart.
//
// Caller has ALREADY verified capacity/solvency/tier/window/dedup checks
// pass; this function only performs the writes. Recomputes
// day/day_entitlement/day_count fresh internally (not passed in) because a
// release can land on a different ledger day than the original submission
// attempt: reusing stale values would double-count against the wrong day
// or (worse) the wrong one entirely.
// -----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn admit_claim(
    env: &Env,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    claim_id: &BytesN<32>,
    entitlement: i128,
    tier: u32,
    hack_timestamp: u64,
    stake_record: &mut StakeRecord,
    count_toward_oracle_limit: bool,
) {
    let day = current_day(env);
    let (day_entitlement, day_count) = storage::get_daily_entitlement(env, day);
    storage::set_daily_entitlement(
        env,
        day,
        day_entitlement + entitlement,
        day_count + if count_toward_oracle_limit { 1 } else { 0 },
    );
    storage::set_total_allocated(env, storage::get_total_allocated(env) + entitlement);

    stake_record.active_claim_id = Some(claim_id.clone());
    // Harmless no-op on the fresh-submission path (already None); clears
    // the wallet's queue slot on the release path.
    stake_record.reserved_claim_id = None;

    let now_ledger = env.ledger().sequence();
    let gate_met = now_ledger.saturating_sub(stake_record.staked_at_ledger) >= TIME_GATE_LEDGERS;

    let mut claim = Claim {
        wallet: wallet.clone(),
        tx_hash: tx_hash.clone(),
        hack_timestamp,
        entitlement,
        streamed: 0,
        stake: stake_record.amount,
        cooldown_ends_ledger: 0,
        vesting_ends_ledger: 0,
        total_staked_snapshot: 0,
        tier,
        status: ClaimStatus::PendingTime,
        approve_deadline_ledger: 0,
        last_collected_ledger: 0,
    };

    // CHANGED 2026-07-22: meeting the gate no longer auto-activates
    // (forfeits/burns/starts cooldown), it moves the claim to
    // AwaitingApproval, where the staker has APPROVE_WINDOW_LEDGERS to
    // actively call approve_claim themselves (Rule A).
    if gate_met {
        claim.status = ClaimStatus::AwaitingApproval;
        claim.approve_deadline_ledger = now_ledger + APPROVE_WINDOW_LEDGERS;
    }

    storage::set_stake(env, wallet, stake_record);
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);
}


// -----------------------------------------------------------------------
// submit_claim
// -----------------------------------------------------------------------

/// Caller must literally be the oracle or the admin (this is the oracle's
/// backend service calling directly once its off-chain scanner finishes,
/// or the admin acting as a manual fallback, not a relayer forwarding a
/// third party's signed blob). Validation order matches KB §1b exactly.
///
/// CHANGED for T2/D1: takes `deadline` + `signature`, and when the caller
/// IS the oracle the contract now verifies an Ed25519 signature over the
/// verdict on-chain. This is V8's belt-and-braces model ported exactly
/// (`SAFUPoolV8.sol:407` caller check AND `:413` signature check), not
/// either/or:
///
/// - `caller.require_auth()` is KEPT. Stellar's docs are explicit that
///   "replay protection is implemented in the host, so there is normally no
///   need for a contract to manage its own nonces", that protection
///   attaches to the authorization entry, and a detached signed payload
///   does NOT inherit it. Dropping host auth would mean hand-rolling replay
///   protection with a double-payout as the failure mode.
/// - The signature is what makes the verdict *attributable* to the oracle's
///   offline key rather than merely to whoever currently holds the oracle
///   Address's signing rights.
///
/// Accepted cost, stated plainly: because host auth is retained, the oracle
/// must submit its own claims, no relayer or third-party submission. Not
/// currently used, so nothing is lost today.
///
/// The admin fallback path is untouched and requires no signature, exactly
/// as in V8 (`:413` is gated on `msg.sender == oracle`). `deadline` and
/// `signature` are ignored on that path.
///
/// Replay: already a no-op independently of the signature layer.
/// `compute_claim_id(wallet, tx_hash)` + the `ClaimAlreadyExists` guard
/// below mean the same wallet+tx can never be claimed twice, the claim-id
/// IS the nonce. `deadline` and the revocation list are defence in depth,
/// and specifically buy the ability to cancel a signed-but-not-yet-submitted
/// approval; they are not the primary replay control.
#[allow(clippy::too_many_arguments)]
pub fn submit_claim(
    env: &Env,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: i128,
    tier: u32,
    hack_timestamp: u64,
    deadline: u64,
    signature: &BytesN<64>,
) -> Result<BytesN<32>, PoolError> {
    storage::require_not_paused(env)?;

    // Pre-audit hardening (2026-09-23): oracle only. The admin's unsigned
    // manual path is removed: one admin key alone could otherwise file claims
    // with no attestation. Manual corrections use the 2-of-2 override.
    let oracle = storage::get_oracle(env);
    if caller != &oracle {
        return Err(PoolError::CallerNotOracleOrAdmin);
    }
    caller.require_auth();

    if entitlement <= 0 {
        return Err(PoolError::EntitlementNotPositive);
    }
    tier_ratio(tier)?; // errors on invalid tier

    // Oracle path only: matches V8's `if (msg.sender == oracle)` at `:413`,
    // and sits at the same point in the validation order: after the caller
    // and basic param checks, before any stake lookup, and well before any
    // storage write.
    if caller == &oracle {
        verify_oracle_signature(
            env,
            wallet,
            tx_hash,
            entitlement,
            tier,
            hack_timestamp,
            deadline,
            signature,
        )?;
    }

    let mut stake_record: StakeRecord =
        storage::get_stake(env, wallet).ok_or(PoolError::NoStake)?;
    if stake_record.amount <= 0 {
        return Err(PoolError::NoActiveStake);
    }
    if stake_record.withdrawn {
        return Err(PoolError::AlreadyWithdrawn);
    }
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }
    if stake_record.active_claim_id.is_some() {
        return Err(PoolError::ClaimAlreadyActiveForStake);
    }
    // T3 (2026-08-24): a wallet can't submit a second, independent claim
    // while one is sitting Reserved, same one-claim-per-wallet invariant
    // active_claim_id already enforces for live claims. A resubmission of
    // the SAME wallet+tx_hash instead hits ClaimAlreadyExists below, once
    // claim_id is computed: this guard only stops a genuinely different
    // claim from stacking on top of a queued one.
    if stake_record.reserved_claim_id.is_some() {
        return Err(PoolError::ClaimAlreadyQueued);
    }

    let cap = tier_cap(stake_record.amount, tier)?;
    if entitlement > cap {
        return Err(PoolError::EntitlementExceedsTierCap);
    }

    let now_ts = env.ledger().timestamp();
    if hack_timestamp > now_ts {
        return Err(PoolError::HackTimestampInFuture);
    }
    if hack_timestamp < stake_record.staked_at_timestamp {
        return Err(PoolError::HackPredatesStake);
    }
    if now_ts > hack_timestamp + CLAIM_WINDOW_SECONDS {
        return Err(PoolError::ClaimWindowExpired);
    }

    // v1: solvency against capacity (stakers + matured backers).
    let capacity = storage::get_capacity(env);
    let total_allocated = storage::get_total_allocated(env);
    let insolvent = total_allocated + entitlement > capacity;

    let day = current_day(env);
    let (day_entitlement, day_count) = storage::get_daily_entitlement(env, day);
    let stress_capped = day_entitlement + entitlement > stress_cap(env);

    let is_oracle_caller = caller == &oracle;

    // Oracle's own daily submission-count throttle. Evaluated here rather
    // than after the queue decision, because as of the 2026-08-24 scope
    // widening it is one of the three conditions that QUEUE rather than
    // reject (see the block comment below).
    let oracle_rate_limited = if is_oracle_caller {
        let total_stakers = storage::get_total_stakers(env);
        let limit = (total_stakers / 10).max(1);
        day_count >= limit
    } else {
        false
    };

    let claim_id = compute_claim_id(env, wallet, tx_hash);
    if let Some(existing) = storage::get_claim(env, &claim_id) {
        if existing.status != ClaimStatus::Unused {
            return Err(PoolError::ClaimAlreadyExists);
        }
    }

    // T3 (2026-08-24): queue instead of hard-reject. All three conditions
    // below share one property: the claim is GENUINE and merely un-admittable
    // right now for a reason outside the claimant's control. That is exactly
    // what is re-triable, unlike a logic error (bad tier, expired window,
    // hack predating the stake), which still hard-rejects above. The stored
    // claim pins exactly what was submitted; nothing is re-derived on release
    // (see `try_release_queued_claim`).
    //
    // `oracle_rate_limited` was ADDED to this set 2026-08-24, having first
    // been deliberately excluded the same day. The exclusion reasoned that it
    // is a flat anti-abuse counter rather than a capacity signal, which is
    // true but turned out to be the wrong test. The right test is this
    // one: a genuine hack that cannot be admitted because of system
    // logic must not be silently lost. This limit is `total_stakers / 10`,
    // so it bites hardest precisely during a mass incident (a bridge failure
    // hitting many stakers at once), the moment when losing genuine claims
    // is least acceptable.
    //
    // Security note, stated rather than assumed: queuing does weaken this
    // counter as a throttle, because a released claim deliberately does not
    // count against it. What still bounds a compromised oracle is unchanged
    // and stronger: the per-wallet `reserved_claim_id` guard caps it at one
    // queued claim per wallet, `stress_cap` bounds total entitlement admitted
    // per day at RELEASE time, the solvency invariant bounds it absolutely,
    // and every submission still needs a valid oracle signature. The counter
    // was always the soft outer layer, never the binding control.
    if insolvent || stress_capped || oracle_rate_limited {
        let claim = Claim {
            wallet: wallet.clone(),
            tx_hash: tx_hash.clone(),
            hack_timestamp,
            entitlement,
            streamed: 0,
            stake: stake_record.amount,
            cooldown_ends_ledger: 0,
            vesting_ends_ledger: 0,
            total_staked_snapshot: 0,
            tier,
            status: ClaimStatus::Reserved,
            approve_deadline_ledger: 0,
            last_collected_ledger: 0,
        };
        stake_record.reserved_claim_id = Some(claim_id.clone());
        storage::set_stake(env, wallet, &stake_record);
        storage::set_claim(env, &claim_id, &claim);
        storage::bump_instance_ttl(env);

        ClaimQueued {
            wallet: wallet.clone(),
            claim_id: claim_id.clone(),
            entitlement,
        }
        .publish(env);

        return Ok(claim_id);
    }

    admit_claim(
        env,
        wallet,
        tx_hash,
        &claim_id,
        entitlement,
        tier,
        hack_timestamp,
        &mut stake_record,
        is_oracle_caller,
    );

    ClaimSubmitted {
        wallet: wallet.clone(),
        claim_id: claim_id.clone(),
        entitlement,
    }
    .publish(env);

    Ok(claim_id)
}

// -----------------------------------------------------------------------
// try_release_queued_claim / expire_queued_claim: T3 (2026-08-24).
// Permissionless, mirroring `expire_pending_approval`/`expire_stale_claim`'s
// existing pattern: Soroban has no native scheduler, so an external caller
// (staker, keeper, anyone) has to trigger these state transitions. Neither
// is time-based-auto-execute: release only happens if a fresh re-check of
// the SAME blockers against CURRENT state actually passes.
// -----------------------------------------------------------------------

pub fn try_release_queued_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;

    let claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchQueuedClaim)?;
    if claim.status != ClaimStatus::Reserved {
        return Err(PoolError::NoSuchQueuedClaim);
    }

    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;

    // One-claim-per-wallet, re-asserted at RELEASE time, the same invariant
    // `submit_claim` and `execute_override` enforce, and the reason this
    // check cannot be skipped just because it passed at queue time: a
    // `Reserved` claim deliberately does not set `active_claim_id`, so a
    // 2-of-2 `execute_override` can create a DIFFERENT live claim for this
    // wallet while this one sits queued. Releasing without re-checking would
    // then produce two independently-payable claims against one stake,
    // exactly what the 2026-07-22 "Bug 2 fix" closed for the override path.
    // Found 2026-08-24 by the T3 audit pass, not by a test.
    if let Some(active_id) = &stake_record.active_claim_id {
        if active_id != claim_id {
            return Err(PoolError::WalletHasDifferentActiveClaim);
        }
    }
    // Pre-audit hardening (H1): release only against the SAME live stake the
    // claim was queued on. The stake record's own queue pointer is what ties
    // the two together; a withdrawn or replaced stake has nothing to forfeit.
    if stake_record.reserved_claim_id.as_ref() != Some(claim_id)
        || stake_record.amount <= 0
        || stake_record.withdrawn
    {
        return Err(PoolError::QueuedClaimStakeChanged);
    }
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }

    let capacity = storage::get_capacity(env);
    let total_allocated = storage::get_total_allocated(env);
    if total_allocated + claim.entitlement > capacity {
        return Err(PoolError::QueueReleaseNotYetEligible);
    }

    let day = current_day(env);
    let (day_entitlement, _) = storage::get_daily_entitlement(env, day);
    if day_entitlement + claim.entitlement > stress_cap(env) {
        return Err(PoolError::QueueReleaseNotYetEligible);
    }

    admit_claim(
        env,
        &claim.wallet,
        &claim.tx_hash,
        claim_id,
        claim.entitlement,
        claim.tier,
        claim.hack_timestamp,
        &mut stake_record,
        // Never counts toward the oracle's own daily submission-count
        // limit: this call can come from anyone, not necessarily the
        // oracle re-submitting, and the original submission already
        // skipped that check entirely by queuing instead of hard-failing.
        false,
    );

    ClaimQueueReleased {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
    }
    .publish(env);
    Ok(())
}

/// A queued claim whose window ran out before capacity ever freed. Releases
/// the wallet's reservation slot so a fresh incident can be submitted. No
/// funds/allocation to release: a Reserved claim never touched
/// `total_allocated` in the first place.
pub fn expire_queued_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;

    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchQueuedClaim)?;
    if claim.status != ClaimStatus::Reserved {
        return Err(PoolError::NoSuchQueuedClaim);
    }

    let now_ts = env.ledger().timestamp();
    if now_ts <= claim.hack_timestamp + CLAIM_WINDOW_SECONDS {
        return Err(PoolError::QueueNotYetExpired);
    }

    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    // Clear only this claim's own slot, never another claim's.
    if stake_record.reserved_claim_id.as_ref() == Some(claim_id) {
        stake_record.reserved_claim_id = None;
        storage::set_stake(env, &claim.wallet, &stake_record);
    }

    claim.status = ClaimStatus::Expired;
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimQueueExpired {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// unlock_pending_claim: permissionless by design (staker, oracle, or
// admin can all call it once the time gate is met; no require_auth).
//
// CHANGED 2026-07-22: no longer activates (forfeits/burns/starts cooldown).
// It moves PendingTime -> AwaitingApproval, same as submit_claim's
// already-gate-met branch. The staker still has to separately call
// approve_claim (Rule A) before anything is forfeited.
// -----------------------------------------------------------------------

pub fn unlock_pending_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;

    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    if claim.status != ClaimStatus::PendingTime {
        return Err(PoolError::ClaimNotPending);
    }

    let stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    let now_ledger = env.ledger().sequence();
    if now_ledger < stake_record.staked_at_ledger + TIME_GATE_LEDGERS {
        return Err(PoolError::TimeGateNotMet);
    }

    claim.status = ClaimStatus::AwaitingApproval;
    claim.approve_deadline_ledger = now_ledger + APPROVE_WINDOW_LEDGERS;

    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimUnlocked {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// approve_claim: NEW 2026-07-22, Rule A. The ONE action, alongside
// claim_stream, that the staker themselves authorizes. This is the
// genuine "walk away or pay the points cost" decision point: burns the
// wallet's entire lifetime points_balance, then forfeits the stake and
// starts the cooldown/vesting clock exactly as the old auto-activation
// used to. Must be called within APPROVE_WINDOW_LEDGERS of entering
// AwaitingApproval, or expire_pending_approval sweeps the reservation
// back to the pool instead.
// -----------------------------------------------------------------------

pub fn approve_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;

    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    claim.wallet.require_auth();

    if claim.status != ClaimStatus::AwaitingApproval {
        return Err(PoolError::ClaimNotAwaitingApproval);
    }
    let now_ledger = env.ledger().sequence();
    if now_ledger > claim.approve_deadline_ledger {
        return Err(PoolError::ApprovalWindowExpired);
    }

    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }
    // Pre-audit hardening (H1): the stake must still be the live one this
    // claim was admitted against. Forfeiting an already-emptied record
    // would pay the claim for nothing.
    if stake_record.withdrawn || stake_record.active_claim_id.as_ref() != Some(claim_id) {
        return Err(PoolError::ClaimStakeMismatch);
    }

    let points_burned = activate_claim(env, &mut stake_record, &mut claim);

    storage::set_stake(env, &claim.wallet, &stake_record);
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimApproved {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
        points_burned,
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// expire_pending_approval: NEW 2026-07-22, Rule A sweep. Permissionless,
// mirrors unlock_pending_claim's pattern (anyone can trigger a time-based
// transition; Soroban has no native scheduler, so this relies on an
// external caller: flagged operationally in the eng review, recommend
// the oracle backend/a keeper script owns calling this in production).
// Nothing was ever forfeited in AwaitingApproval, so there's no stake to
// restore: just release the reservation and unblock the wallet.
// -----------------------------------------------------------------------

pub fn expire_pending_approval(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    if claim.status != ClaimStatus::AwaitingApproval {
        return Err(PoolError::ClaimNotAwaitingApproval);
    }
    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    // Found in adversarial re-review 2026-07-22, after the initial
    // implementation: blocker #1's fix only reset the clock on unsuspend.
    // It never stopped a suspended staker's deadline from expiring
    // WHILE still suspended. Without this check, staying suspended past
    // the deadline (never explicitly unsuspended) would sweep the
    // reservation away regardless: exactly the unfairness blocker #1 was
    // meant to prevent. Blocking the sweep entirely while suspended closes
    // this: it can only ever expire after an explicit unsuspend, which
    // itself grants a fresh full window.
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }
    let now_ledger = env.ledger().sequence();
    if now_ledger <= claim.approve_deadline_ledger {
        return Err(PoolError::ApprovalWindowNotExpired);
    }

    let total_allocated = storage::get_total_allocated(env);
    storage::set_total_allocated(env, total_allocated.saturating_sub(claim.entitlement));

    stake_record.active_claim_id = None;
    storage::set_stake(env, &claim.wallet, &stake_record);

    claim.status = ClaimStatus::Expired;
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimExpired {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
        released: claim.entitlement,
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// expire_stale_claim: NEW 2026-07-22, Rule B sweep. Permissionless, same
// operational caveat as expire_pending_approval above. Releases whatever
// entitlement remains uncollected after COLLECTION_INACTIVITY_LEDGERS of
// zero claim_stream activity. Forfeiture already happened at approval, so
// (unlike expire_pending_approval) there's no stake_record to unblock,
// active_claim_id is cleared for storage hygiene only, harmless either
// way since `withdrawn=true` already blocks every relevant path.
// -----------------------------------------------------------------------

pub fn expire_stale_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    if claim.status != ClaimStatus::Active {
        return Err(PoolError::ClaimNotActive);
    }
    if claim.streamed >= claim.entitlement {
        return Err(PoolError::ClaimFullyStreamed);
    }
    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    // Same fairness fix as expire_pending_approval above, found in the
    // same adversarial re-review: a suspended staker's Rule B clock must
    // not be allowed to expire while they're still frozen out of
    // collecting.
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }
    let now_ledger = env.ledger().sequence();
    if now_ledger.saturating_sub(claim.last_collected_ledger) <= COLLECTION_INACTIVITY_LEDGERS {
        return Err(PoolError::ClaimNotStale);
    }

    let remaining = claim.entitlement - claim.streamed;
    let total_allocated = storage::get_total_allocated(env);
    storage::set_total_allocated(env, total_allocated.saturating_sub(remaining));

    stake_record.active_claim_id = None;
    storage::set_stake(env, &claim.wallet, &stake_record);

    claim.status = ClaimStatus::Expired;
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimExpired {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
        released: remaining,
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// claim_stream: pull-payment, staker-authorized (the ONE claims
// function the staker/wallet itself authorizes; everything else is
// oracle/admin/permissionless).
// -----------------------------------------------------------------------

pub fn claim_stream(
    env: &Env,
    claim_id: &BytesN<32>,
    beneficiary: &Address,
) -> Result<i128, PoolError> {
    storage::require_not_paused(env)?;

    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    claim.wallet.require_auth();

    if claim.status != ClaimStatus::Active {
        return Err(PoolError::ClaimNotActive);
    }
    let now_ledger = env.ledger().sequence();
    if now_ledger < claim.cooldown_ends_ledger {
        return Err(PoolError::CooldownNotPassed);
    }
    if claim.streamed >= claim.entitlement {
        return Err(PoolError::ClaimFullyStreamed);
    }

    let stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;
    if stake_record.suspended {
        return Err(PoolError::StakeSuspended);
    }
    let expected_hash = env.crypto().sha256(&beneficiary.to_xdr(env)).to_bytes();
    if expected_hash != stake_record.beneficiary_hash {
        return Err(PoolError::WrongBeneficiary);
    }

    let elapsed_end = now_ledger.min(claim.vesting_ends_ledger);
    let elapsed = elapsed_end.saturating_sub(claim.cooldown_ends_ledger) as i128;
    // v1 (2026-09-22): divide by THIS claim's own vesting window, fixed when
    // it activated, never by the live setting. Vesting becomes adjustable in
    // v1, and a setting change must only affect claims that activate after
    // it; reading the live value here would re-price every open stream.
    let vesting_len = claim.vesting_ends_ledger.saturating_sub(claim.cooldown_ends_ledger) as i128;
    let vested_total = if vesting_len == 0 {
        claim.entitlement
    } else {
        claim.entitlement * elapsed / vesting_len
    };
    let claimable = vested_total - claim.streamed;
    if claimable <= 0 {
        return Err(PoolError::NothingVested);
    }

    let day = current_day(env);
    let daily_outflow_so_far = storage::get_daily_outflow(env, day);
    let cap_base = storage::get_capacity(env).max(claim.total_staked_snapshot);
    let bps = dynamic_outflow_bps(env, cap_base);
    let cap = cap_base * bps / BPS_DENOMINATOR;
    // Bug 4 fix (eng review 2026-07-22): was `(cap - daily_outflow_so_far)
    // .max(0)`: under this workspace's `overflow-checks = true`, the
    // subtraction itself panics before `.max(0)` ever runs if
    // daily_outflow_so_far > cap (a real, reachable case, the cap can
    // shrink mid-day as utilization rises from other claims), crashing a
    // legitimate staker's routine call instead of gracefully returning
    // "try again tomorrow." saturating_sub clamps within the subtraction
    // itself, so there's nothing left for a stale `.max(0)` to (fail to) do.
    let available_today = cap.saturating_sub(daily_outflow_so_far);
    let transfer_amount = claimable.min(available_today);
    if transfer_amount <= 0 {
        return Err(PoolError::DailyOutflowCapReached);
    }

    // D2: the solvency gate (`submit_claim`) and the outflow cap above both
    // reason over `total_staked` / `total_allocated`, accounting figures
    // that stay correct while XLM sits in the yield vault, which is exactly
    // why D2 required no change to either. The question they cannot answer
    // is whether the contract physically holds the XLM right now. That is a
    // liquidity question, not a solvency one, and it gets its own check
    // here so a short balance reports as a typed error a staker can act on
    // rather than an opaque trap. Checked before any state mutation.
    // v1: `pull_for_payment` redeems from the vault first if cash is short.
    crate::vault::pull_for_payment(env, transfer_amount)?;

    claim.streamed += transfer_amount;
    // Rule B: any successful collection resets the inactivity clock.
    claim.last_collected_ledger = now_ledger;
    if claim.streamed >= claim.entitlement {
        claim.status = ClaimStatus::Completed;
    }
    // Bug 1 fix (eng review 2026-07-22): release total_allocated
    // incrementally as money actually leaves, not only at Completed or
    // via cancel/override. Every claim that pays out in full used to
    // permanently inflate total_allocated with nothing ever decrementing
    // it back: eventually the solvency check in submit_claim would
    // reject all new legitimate claims even with ample fresh capital.
    // This shape (decrement per transfer) is also what makes
    // expire_stale_claim's release math correct: by the time it runs,
    // total_allocated already reflects exactly `entitlement - streamed`
    // for this claim, the same pattern cancel_claim/execute_override use.
    let total_allocated = storage::get_total_allocated(env);
    storage::set_total_allocated(env, total_allocated.saturating_sub(transfer_amount));
    storage::set_daily_outflow(env, day, daily_outflow_so_far + transfer_amount);
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimStreamed {
        wallet: claim.wallet.clone(),
        amount: transfer_amount,
    }
    .publish(env);

    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(&env.current_contract_address(), beneficiary, &transfer_amount);

    Ok(transfer_amount)
}

// -----------------------------------------------------------------------
// cancel_claim: admin-only, false-positive reversal.
// -----------------------------------------------------------------------

pub fn cancel_claim(env: &Env, claim_id: &BytesN<32>) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    let mut claim: Claim = storage::get_claim(env, claim_id).ok_or(PoolError::NoSuchClaim)?;
    // CHANGED 2026-07-22 (eng review blocker #4): AwaitingApproval added.
    // Nothing forfeits during AwaitingApproval (same as PendingTime), so
    // it falls into the same no-penalty branch below, without this, admin
    // could not cancel a false positive sitting in the new approval window.
    if claim.status != ClaimStatus::Active
        && claim.status != ClaimStatus::PendingTime
        && claim.status != ClaimStatus::AwaitingApproval
    {
        return Err(PoolError::ClaimNotCancellable);
    }

    let unstreamed = claim.entitlement - claim.streamed;
    let total_allocated = storage::get_total_allocated(env);
    // Bug 4 fix: saturating_sub, not subtract-then-.max(0), see claim_stream
    // for the full explanation of why the old pattern was dead code.
    storage::set_total_allocated(env, total_allocated.saturating_sub(unstreamed));

    let mut stake_record: StakeRecord =
        storage::get_stake(env, &claim.wallet).ok_or(PoolError::NoStake)?;

    // Active: the stake was already forfeited at activation, restore it
    // and apply the 365-day penalty lock. PendingTime/AwaitingApproval:
    // the stake was never forfeited (no agency over the oracle's
    // submission, and no agency over an admin's own gate-met transition
    // either), nothing to restore, no penalty, just unblock withdrawal
    // below.
    if claim.status == ClaimStatus::Active {
        stake_record.withdrawn = false;
        // The approval is undone, so the lifetime restake ban lifts with it.
        stake_record.claim_approved = false;
        // No `stake_record.amount = claim.stake` here, amount was never
        // zeroed by forfeiture (see activate_claim's doc comment), so
        // it's already correct; V8's own cancelClaim doesn't touch it
        // either, confirmed by reading the source directly.
        stake_record.penalty_locked_until_ledger = env.ledger().sequence() + PENALTY_LOCK_LEDGERS;
        storage::set_total_staked(env, storage::get_total_staked(env) + claim.stake);
        storage::set_total_stakers(env, storage::get_total_stakers(env) + 1);
    }

    stake_record.active_claim_id = None;
    storage::set_stake(env, &claim.wallet, &stake_record);

    claim.status = ClaimStatus::Cancelled;
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    ClaimCancelled {
        wallet: claim.wallet.clone(),
        claim_id: claim_id.clone(),
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// 2-of-2 override: admin + coSigner, each submits identical params once;
// the second matching call executes. Bypasses the oracle/time-gate path
// entirely; the multi-sig itself is the security gate.
// -----------------------------------------------------------------------

pub fn approve_override(
    env: &Env,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: i128,
    tier: u32,
) -> Result<(), PoolError> {
    storage::require_not_paused(env)?;

    let admin = storage::get_admin(env);
    let co_signer = storage::get_co_signer(env);
    if caller != &admin && caller != &co_signer {
        return Err(PoolError::CallerNotAdminOrCoSigner);
    }
    caller.require_auth();

    if entitlement <= 0 {
        return Err(PoolError::EntitlementNotPositive);
    }
    tier_ratio(tier)?;

    let claim_id = compute_claim_id(env, wallet, tx_hash);

    let mut req: OverrideRequest =
        storage::get_override(env, &claim_id).unwrap_or(OverrideRequest {
            wallet: wallet.clone(),
            tx_hash: tx_hash.clone(),
            entitlement,
            tier,
            owner_approver: None,
            co_signer_approver: None,
        });

    if req.entitlement != entitlement || req.tier != tier {
        return Err(PoolError::OverrideParamsMismatch);
    }

    if caller == &admin {
        req.owner_approver = Some(admin.clone());
    }
    if caller == &co_signer {
        req.co_signer_approver = Some(co_signer.clone());
    }
    OverrideApproved {
        wallet: wallet.clone(),
        claim_id: claim_id.clone(),
        approver: caller.clone(),
        entitlement,
        tier,
    }
    .publish(env);

    // Ready only if BOTH approvers still match the CURRENT admin/coSigner.
    // An approval recorded at some earlier point is not enough. A rotation
    // via set_co_signer/transfer_admin between the two approvals makes a
    // stale approver no longer equal the current address, so `ready`
    // correctly stays false until the NEW party re-approves. Degenerate
    // case unchanged: admin and coSigner are the same address, one
    // approval suffices.
    let ready = (req.owner_approver == Some(admin.clone())
        && req.co_signer_approver == Some(co_signer.clone()))
        || co_signer == admin;

    if ready {
        execute_override(env, &claim_id, &req)?;
        // Deleted, not reset: matches V8's `_executeOverride`, which
        // does `delete pendingOverrides[claimId]` unconditionally before
        // anything else runs. This also means cancel_pending_override
        // doesn't need a separate "already executed" check: looking up a
        // deleted request naturally fails with "no pending override",
        // the same error V8 gets from the same mechanism.
        storage::remove_override(env, &claim_id);
    } else {
        storage::set_override(env, &claim_id, &req);
    }
    Ok(())
}

fn execute_override(env: &Env, claim_id: &BytesN<32>, req: &OverrideRequest) -> Result<(), PoolError> {
    let existing = storage::get_claim(env, claim_id);
    // Bug 3 fix (eng review 2026-07-22): if this is a re-execution/
    // correction of an already-Active, already-partially-streamed claim,
    // carry its `streamed` amount forward. The old code always hard-coded
    // a fresh claim record's `streamed` to 0, so a correction let the
    // beneficiary collect the new entitlement ON TOP of what they'd
    // already received under the old one, an overpayment.
    let mut carried_streamed: i128 = 0;
    if let Some(ref c) = existing {
        if c.status == ClaimStatus::Completed {
            return Err(PoolError::ClaimAlreadyCompleted);
        }
        // Only release when the PRIOR claim actually held a live
        // reservation (Active or PendingTime), verified against V8's
        // `_executeOverride` directly: it gates this release on
        // `prevStatus == 1 || prevStatus == 5` specifically, not "any
        // non-completed status." A Cancelled claim's reservation was
        // already released by cancel_claim itself; releasing it AGAIN
        // here (the original, unverified version of this function did)
        // would double-subtract total_allocated for an override
        // re-targeting a previously-cancelled wallet+tx_hash pair,
        // silently masked by the .max(0) clamp rather than caught.
        // AwaitingApproval added 2026-07-22 alongside PendingTime, same
        // "nothing forfeited yet" treatment.
        if c.status == ClaimStatus::Active
            || c.status == ClaimStatus::PendingTime
            || c.status == ClaimStatus::AwaitingApproval
        {
            let unstreamed = c.entitlement - c.streamed;
            let total_allocated = storage::get_total_allocated(env);
            // Bug 4 fix: saturating_sub, not subtract-then-.max(0).
            storage::set_total_allocated(env, total_allocated.saturating_sub(unstreamed));
        }
        if c.status == ClaimStatus::Active {
            carried_streamed = c.streamed;
        }
    }

    let mut stake_record: StakeRecord =
        storage::get_stake(env, &req.wallet).ok_or(PoolError::NoStake)?;

    // Bug 2 fix (eng review 2026-07-22): enforce the one-wallet-one-claim
    // invariant here too: submit_claim already refused a second claim
    // while a wallet had one active, but execute_override had no
    // equivalent guard, so an override under a DIFFERENT tx_hash than an
    // already-in-flight claim could create two independently-payable
    // claims against one forfeited stake. A re-execution of THIS SAME
    // claim_id (correcting terms, resetting cooldown, etc.) is still
    // allowed: only a genuinely different, conflicting claim is blocked.
    if let Some(active_id) = &stake_record.active_claim_id {
        if active_id != claim_id {
            return Err(PoolError::WalletHasDifferentActiveClaim);
        }
    }

    // Reads live: amount is never zeroed by forfeiture (activate_claim's
    // doc comment), so this needs no withdrawn-branch fallback to a prior
    // claim's remembered `stake` field. Matches V8's own `_executeOverride`
    // reading `stakes[wallet_].amount` directly.
    let original_stake_amount = stake_record.amount;
    if original_stake_amount <= 0 {
        return Err(PoolError::NoStakeAmountForOverride);
    }

    let cap = tier_cap(original_stake_amount, req.tier)?;
    if req.entitlement > cap {
        return Err(PoolError::EntitlementExceedsTierCap);
    }

    // If this stake was already forfeited (a same-claim-id re-execution,
    // e.g. resetting cooldown with identical terms), that principal left
    // total_staked on the FIRST execution and is now earmarked reserve
    // capacity for exactly this claim, not fresh pool capacity being
    // drawn down again. Add it back for this check only, so re-execution
    // is evaluated against the same pool view the first execution saw,
    // found via a failing test (a same-params re-approval incorrectly
    // reported insolvent because total_staked no longer included this
    // stake's own already-forfeited principal).
    let capacity = storage::get_capacity(env);
    let effective_total_staked = if stake_record.withdrawn {
        capacity + original_stake_amount
    } else {
        capacity
    };
    let total_allocated = storage::get_total_allocated(env);
    if total_allocated + req.entitlement > effective_total_staked {
        return Err(PoolError::Insolvent);
    }
    storage::set_total_allocated(env, total_allocated + req.entitlement);

    let mut claim = Claim {
        wallet: req.wallet.clone(),
        tx_hash: req.tx_hash.clone(),
        hack_timestamp: env.ledger().timestamp(),
        entitlement: req.entitlement,
        streamed: carried_streamed,
        stake: original_stake_amount,
        cooldown_ends_ledger: 0,
        vesting_ends_ledger: 0,
        total_staked_snapshot: 0,
        tier: req.tier,
        status: ClaimStatus::Active,
        approve_deadline_ledger: 0,
        last_collected_ledger: 0,
    };

    // The 2-of-2 override bypasses the oracle/time-gate path entirely, AND
    // (2026-07-22) the new staker-approval gate too, admin+coSigner
    // consensus IS the security gate, no separate staker approval is
    // required. Activation (including the points burn) happens directly
    // here, same as it always has for a fresh forfeiture.
    // T3 fix (2026-08-24): capture the burn, same as `approve_claim` does
    // for `ClaimApproved` (`claim.rs:659,668`), `activate_claim` already
    // computes and burns points here, this just stops discarding the
    // return value. The re-execution branch burns nothing NEW (points
    // were already burned on the original execution that set
    // `withdrawn = true`), so it reports 0, not a phantom second burn.
    let points_burned = if !stake_record.withdrawn {
        activate_claim(env, &mut stake_record, &mut claim)
    } else {
        let now_ledger = env.ledger().sequence();
        claim.cooldown_ends_ledger = now_ledger + crate::settings::get(env, crate::settings::SettingKey::CooldownLedgers) as u32;
        claim.vesting_ends_ledger = claim.cooldown_ends_ledger + crate::settings::get(env, crate::settings::SettingKey::VestingLedgers) as u32;
        claim.total_staked_snapshot = storage::get_capacity(env);
        claim.last_collected_ledger = claim.cooldown_ends_ledger;
        0
    };
    stake_record.active_claim_id = Some(claim_id.clone());
    // T3 (2026-08-24): if this override is taking over the wallet's OWN
    // queued claim, release the queue slot with it. Without this the record
    // becomes Active while `reserved_claim_id` still points at it, and
    // `expire_queued_claim` can never clear that pointer (it requires status
    // == Reserved), permanently blocking the wallet from queuing again.
    // A queued claim under a DIFFERENT id is deliberately left alone here,
    // it stays independently expirable, and `try_release_queued_claim`'s
    // active-claim guard stops it ever becoming a second live claim.
    if stake_record.reserved_claim_id.as_ref() == Some(claim_id) {
        stake_record.reserved_claim_id = None;
    }

    storage::set_stake(env, &req.wallet, &stake_record);
    storage::set_claim(env, claim_id, &claim);
    storage::bump_instance_ttl(env);

    OverrideExecuted {
        wallet: req.wallet.clone(),
        claim_id: claim_id.clone(),
        points_burned,
    }
    .publish(env);
    Ok(())
}

/// Verified against the live V8 source 2026-07-14 (was previously
/// approximated: see git history). V8's `cancelPendingOverride` is
/// `onlyOwner`: admin ONLY, not admin-or-coSigner as an earlier draft
/// here assumed. It also just requires the request exists and deletes
/// it; no separate "already executed" branch exists because execution
/// ALSO deletes the request (see approve_override), so a post-execution
/// call here naturally fails with the same "no pending override" error
/// as one that never existed, exactly what `storage::get_override(...)
/// .ok_or(...)` below already does, without needing a Claim-status
/// lookup at all.
pub fn cancel_pending_override(
    env: &Env,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    if caller != &admin {
        return Err(PoolError::CallerNotAdmin);
    }
    admin.require_auth();

    let claim_id = compute_claim_id(env, wallet, tx_hash);
    storage::get_override(env, &claim_id).ok_or(PoolError::NoPendingOverride)?;
    storage::remove_override(env, &claim_id);

    OverrideCancelled {
        claim_id: claim_id.clone(),
    }
    .publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// revoke_approval: D1 (T2). Admin-only. Ported from V8's
// `revokeApproval(bytes32 approvalHash) external onlyOwner` (`:837`).
// -----------------------------------------------------------------------

/// Cancels a signed-but-not-yet-submitted oracle approval.
///
/// **Deliberate shape deviation from V8, and the reason for it.** V8 takes
/// the already-computed `approvalHash` directly. This takes the full
/// approval parameters and recomputes the hash on-chain instead. Two
/// reasons, both specific to Soroban rather than preference:
///
/// 1. The revocation lives in `temporary()` storage and needs a TTL that
///    outlives the approval's `deadline`. Taking the parameters means the
///    contract *knows* that deadline and can enforce the relationship,
///    rather than trusting an admin-supplied number alongside an opaque
///    hash. A typo in a separately-passed deadline would set a TTL that
///    fails to cover the real one and silently un-revoke the approval.
/// 2. Recomputing makes it impossible to revoke a garbage hash that
///    corresponds to no real approval, a mistyped hash on V8 writes a
///    permanent no-op entry with no feedback.
///
/// The window checks mirror `verify_oracle_signature` exactly: revoking an
/// already-expired approval is rejected rather than silently accepted,
/// because such an approval is already dead on `SignatureExpired` and a
/// successful-looking revocation would imply protection it is not providing.
#[allow(clippy::too_many_arguments)]
pub fn revoke_approval(
    env: &Env,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: i128,
    tier: u32,
    hack_timestamp: u64,
    deadline: u64,
) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    if caller != &admin {
        return Err(PoolError::CallerNotAdmin);
    }
    admin.require_auth();

    let now_ts = env.ledger().timestamp();
    if now_ts > deadline {
        return Err(PoolError::SignatureExpired);
    }
    if deadline > now_ts + MAX_APPROVAL_WINDOW_SECONDS {
        return Err(PoolError::SignatureDeadlineTooFar);
    }

    let payload = build_approval_payload(
        env,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        deadline,
    );
    let hash = approval_hash(env, &payload);
    storage::set_approval_revoked(env, &hash);

    ApprovalRevoked {
        wallet: wallet.clone(),
        approval_hash: hash,
    }
    .publish(env);
    Ok(())
}
