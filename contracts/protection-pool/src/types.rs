//! Data model ported from `SAFUPoolV8.sol`.
//!
//! Tranche 1 covered pool/claims mechanics only, excluding V8's per-staker
//! yield-deployment fields (`wstethDeployed`, `totalDeployed`,
//! `totalDeployedETH`), see context/knowledge/smartcontract-soroban.md
//! "scope boundary" note (research-ops repo) for why.
//!
//! Doc corrected 2026-08-17 (7a audit, Finding 6): this header still claimed
//! "no yield venue" after D2 landed. Tranche 2 DOES add yield, and its
//! constants live in this file (`MAX_DEPLOY_BPS`, `DEPLOY_BPS_DENOMINATOR`,
//! see the D2 block below). What remains true is that V8's *per-staker*
//! deployment tranche is still deliberately absent, this contract holds one
//! pooled position and bounds deployment by admin policy instead, which is
//! why V8's inline 100%-deploy could not be ported. D1's oracle-signature
//! constants (`MAX_APPROVAL_WINDOW_SECONDS`, `REVOCATION_TTL_LEDGERS`) are
//! also defined here.

use soroban_sdk::{contracttype, Address, BytesN};

// -----------------------------------------------------------------------
// Time constants: Soroban has no native "days"; ledgers close ~5s apart.
// Mirrors DAY_IN_LEDGERS convention from the Soroban SDK reference used to
// build this contract (context/knowledge/smartcontract-soroban.md).
// -----------------------------------------------------------------------

pub const LEDGERS_PER_DAY: u32 = 17_280;
/// Real calendar time, for absolute-timestamp checks (hack_timestamp
/// sanity, claim window, daily-counter day boundaries), distinct from
/// the LEDGERS_PER_DAY constants above, which drive duration/gate math
/// off `env.ledger().sequence()`. Soroban exposes both `sequence()` (a
/// ledger number) and `timestamp()` (real Unix seconds) on `env.ledger()`;
/// mixing them for their respective purposes is deliberate, not sloppy.
pub const SECONDS_PER_DAY: u64 = 86_400;
/// 30-day window after a hack within which the claim must be submitted,
/// expressed in real seconds (compared against `env.ledger().timestamp()`
/// and the oracle-supplied `hack_timestamp`, both Unix seconds).
pub const CLAIM_WINDOW_SECONDS: u64 = 30 * SECONDS_PER_DAY;
/// Shared percentage-math denominator (tier cap, stress cap, payout
/// outflow bps), same value as STAKE_BPS_DENOMINATOR/TIER_BPS_DENOMINATOR
/// below, kept as its own constant since those two are domain-specific.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// 90-day time gate before a stake is claim-eligible (V8: CLAIM_MIN_DAYS).
/// Production value, restored from T3 for the v1 build (founder decision
/// 2026-09-22). Fixed forever: not one of the adjustable settings.
pub const TIME_GATE_LEDGERS: u32 = 90 * LEDGERS_PER_DAY;
/// Cooldown between claim activation and first payout stream. Production
/// default from T3; becomes an adjustable, bounded setting in v1.
pub const COOLDOWN_LEDGERS: u32 = 7 * LEDGERS_PER_DAY;
/// Linear vesting window, starting at cooldown end (V8: VESTING).
/// Production default from T3; becomes an adjustable, bounded setting in v1.
pub const VESTING_LEDGERS: u32 = 45 * LEDGERS_PER_DAY;
/// 365-day re-stake lock applied only on a false-positive cancel, never on
/// a genuine paid claim (that forfeiture is permanent, do not conflate).
pub const PENALTY_LOCK_LEDGERS: u32 = 365 * LEDGERS_PER_DAY;

// -----------------------------------------------------------------------
// D1 (Tranche 2), on-chain Ed25519 oracle approval verification.
//
// V8 has no equivalent to the two constants below: on EVM, `revokedApprovals`
// is a permanent mapping, so a revocation simply never expires and no
// deadline bound is needed to keep it alive. Soroban's `temporary()` storage
// has a finite TTL, which introduces a failure mode EVM does not have, a
// revocation that expires BEFORE the approval it revokes silently
// un-revokes that approval. These two constants exist to make that failure
// mode unreachable, and the const-assert below is what proves it rather
// than leaving it to convention (eng review, Warning 6 / revocation-TTL
// resolution, `outputs/2026-08-13_plan-eng-review-safu-t2-d1-ed25519.md`).
// -----------------------------------------------------------------------

/// Maximum lifetime of a single oracle approval signature, in real seconds.
/// `submit_claim` rejects any `deadline` further out than this. Bounding it
/// is what lets `REVOCATION_TTL_LEDGERS` provably outlive every deadline a
/// valid approval can carry.
pub const MAX_APPROVAL_WINDOW_SECONDS: u64 = 24 * 3_600;

/// TTL applied to a revocation entry in `temporary()` storage.
///
/// A revocation only has to survive until the approval it revokes expires,
/// after that, `SignatureExpired` rejects the approval anyway, so the
/// revocation is no longer load-bearing and letting it be reclaimed is
/// correct (and keeps revocation state bounded rather than growing forever).
///
/// Note the asymmetry that makes `temporary()` safe here: the entry records
/// a REVOCATION, not a permission. Anyone extending its TTL, which anyone
/// can do on Soroban, permissionlessly, only makes the revocation last
/// LONGER. The failure direction is fail-closed. The one genuine hazard is
/// a TTL shorter than the deadline, which the const-assert below rules out.
pub const REVOCATION_TTL_LEDGERS: u32 = 7 * LEDGERS_PER_DAY;

// The revocation must outlive the approval under every ledger close rate.
// Ledgers nominally close ~5s apart, but that is a network property this
// contract cannot depend on, so the check assumes a pathological 1 second
// per ledger: the worst case for us, since faster ledgers burn TTL faster
// in wall-clock terms. Under that assumption REVOCATION_TTL_LEDGERS covers
// REVOCATION_TTL_LEDGERS seconds; requiring that to exceed the maximum
// approval window makes "revocation outlives deadline" a compile-time fact.
// At the nominal 5s/ledger the real margin is 7 days against a 24h bound.
const _: () = assert!(
    REVOCATION_TTL_LEDGERS as u64 >= MAX_APPROVAL_WINDOW_SECONDS,
    "revocation TTL must outlive the longest legal approval deadline"
);

/// Points burn-on-claim mechanism (locked 2026-07-22, task plan
/// `outputs/2026-07-22_task-plan-safu-points-burn-mechanism.md`, eng review
/// `outputs/2026-07-22_plan-eng-review-safu-points-burn-mechanism.md`).
///
/// Rule A: once a claim's 90-day time gate is met, the staker has this long
/// to actively call `approve_claim` (which burns their full lifetime points
/// balance and starts forfeiture/cooldown/vesting) before the reservation
/// expires back to the pool via the permissionless `expire_pending_approval`.
pub const APPROVE_WINDOW_LEDGERS: u32 = 100 * LEDGERS_PER_DAY;
/// Rule B: once approved and streaming, a single rolling inactivity clock
/// per claim (not per-day-siloed: the vesting model is one continuous
/// running total, so per-day tracking would need new data; a rolling
/// clock resetting on every `claim_stream` call is the simpler, correct
/// choice, per the eng review). If a claim goes this long with zero
/// `claim_stream` activity, whatever remains uncollected sweeps back to
/// the pool via the permissionless `expire_stale_claim`. Anchored at
/// `cooldown_ends_ledger` on approval, never at the approval moment itself.
/// The 7-day mandatory cooldown must never count as staker inactivity
/// (eng review blocker #2).
pub const COLLECTION_INACTIVITY_LEDGERS: u32 = 100 * LEDGERS_PER_DAY;

// -----------------------------------------------------------------------
// D2 (Tranche 2), DeFindex vault yield deployment.
//
// V8 deploys 100% of every stake into Lido inline inside `stakeETH`
// (`SAFUPoolV8.sol:295-303`) and holds no standing buffer, relying on the
// owner to call `provideClaimLiquidity` during the 7-day claim cooldown.
// That is safe on V8 ONLY because V8 tracks deployment per staker
// (`StakeRecord.wstethDeployed`) and unwinds that exact tranche inline
// inside `withdraw()` (`:330-346`), so a withdrawal always has precisely
// its own principal available.
//
// This contract has no per-staker deployment tranche, and, decisively,
// neither of its two withdrawal paths has any cooldown in which an
// operator could react: `stake::withdraw` has no time lock at all, and
// `stake::emergency_exit` is the pause-time escape hatch, so it must work
// exactly when the operator is least able to intervene. Porting V8's
// deploy policy here would therefore break principal withdrawal as well as
// claims.
//
// Locked design (eng review 2026-08-14,
// `outputs/2026-08-14_plan-eng-review-safu-t2-d2-yield-integration.md`):
// deployment is bounded by `deploy_bps` and floored so it can never touch
// already-reserved entitlements. The 2026-08-14 rule "NEVER auto-unwound from
// any user-facing path" was SUPERSEDED 2026-09-24 (founder): the pool now
// pulls from the vault inside a payment when cash is short and pushes idle
// cash in on stake/back, both through `try_` calls that leave no state behind
// on a vault failure. See vault.rs module doc, rule 3.
// -----------------------------------------------------------------------

/// Hard ceiling on `deploy_bps`, enforced in `set_deploy_bps`, an admin
/// cannot configure the pool into V8's 100%-deployed posture even
/// deliberately. Operational recommendation for T2 is 5_000 (50%); this
/// constant is the bound, not the setting.
pub const MAX_DEPLOY_BPS: i128 = 8_000;

/// `deploy_bps` and the vault address both start unset, so every yield
/// path is fail-closed on a fresh deploy: `deploy_bps` reads 0 and
/// `set_vault` has not run, meaning the pool behaves exactly as it did in
/// Tranche 1 until an admin deliberately turns deployment on. This is why
/// D2 requires NO change to `initialize`'s signature, unlike D1, which
/// had to add `oracle_pubkey`.
pub const DEPLOY_BPS_DENOMINATOR: i128 = 10_000;

/// T3 (2026-08-24): max slippage `ensure_liquidity`/`auto_deploy_liquidity`
/// will accept on their own contract-computed redemption/deposit, nothing
/// caller-controlled, per the 2026-08-20 locked design. 500 = 5%. Applies
/// to both directions: a redeem accepts down to 95% of expected XLM, a
/// deposit requires at least 95% of the shares the contract's own
/// last-known rate (`deployed_asset / deployed_shares`) would predict.
pub const MAX_REBALANCE_SLIPPAGE_BPS: i128 = 500;

/// v1 (2026-09-24): the in-path push (`vault::push_idle`, run inside `stake`
/// and `back`) deposits only once the idle amount is at least this share of
/// capacity (100 = 1%), so a stream of small stakes doesn't each pay for a
/// vault call, and the pull/push lines can't bounce off each other.
pub const AUTO_PUSH_MIN_BPS: i128 = 100;

// -----------------------------------------------------------------------
// Yield split: added for the Stellar Pro Hackathon fork (2026-09-18,
// founder decision). `extract_yield` used to send 100% of realised yield to
// treasury. Now it splits: half compounds into every live staker's balance
// (via `YieldIndex`, an Aave-style growing multiplier, NOT a per-user
// share token, which would reopen the classic ERC4626 first-depositor
// donation-attack surface), half becomes protocol-owned money sitting in
// the pool, withdrawable by the admin at any time via `withdraw_yield`
// ("the protocol's own money", founder's words, not auto-transferred the
// moment it is realised).
//
// Deliberately does NOT touch `StakeRecord.amount` or `tier_cap`'s input.
// A stake's ENTITLEMENT ceiling stays anchored to its fixed admission-time
// principal (already bounded by MAX_STAKE_BPS below) forever, yield never
// inflates how much a wallet can be paid on a claim, only how much it can
// withdraw voluntarily. "if the yield+principal is 2% [of pool cap], it
// still caps the protection to 1.25%" (founder).
/// v1 (2026-09-22): DEFAULT of the adjustable `StakerYieldBps` setting.
/// Founder decision: 100% of yield on staker capital goes to stakers.
pub const YIELD_SPLIT_STAKER_BPS: i128 = 10_000;
pub const YIELD_SPLIT_BPS_DENOMINATOR: i128 = 10_000;

/// Fixed-point scale for `YieldIndex`. Chosen with headroom against i128
/// overflow: realistic stroop amounts (<=1e15) times this precision
/// (1e12) stay under 1e27, far inside i128's ~1.7e38 ceiling, even before
/// dividing back down.
pub const YIELD_INDEX_PRECISION: i128 = 1_000_000_000_000;

// -----------------------------------------------------------------------
// Stake bounds: basis points of the configurable pool cap,
// instead of fixed amounts. Decided 2026-07-14 (user): V8's fixed ETH bounds
// (STAKE_MIN=0.01, STAKE_MAX=0.75, MAX_POOL_ETH=60) imply MAX = exactly
// 1.25% of pool cap and MIN = 0.0167% of pool cap, a per-staker
// concentration limit (max) plus a spam-resistance floor (min), not
// arbitrary numbers. Recomputing these as basis-points-of-pool-cap
// instead of fixed amounts makes the same design portable across chains
// with different pool sizes (EVM/Solana/BNB relaunches) without
// re-deriving bounds by hand each time, this is meant to be the reusable
// base pattern, not a Soroban-only fix.
// SET 2026-09-19 (founder, this build): $10 min / $100 max per staker at the
// $100,000 USDC deploy target -- no longer V8's ratio. Both land on clean
// integers at this pool cap (100_000 * 1 / 10_000 = 10, * 10 / 10_000 = 100),
// so no rounding approximation is needed, unlike the superseded V8-matching
// values below.
// -----------------------------------------------------------------------

pub const MIN_STAKE_BPS: i128 = 1; // 0.01% of pool cap -- $10 at a $100,000 pool cap
pub const MAX_STAKE_BPS: i128 = 10; // 0.1% of pool cap -- $100 at a $100,000 pool cap
pub const STAKE_BPS_DENOMINATOR: i128 = 10_000;

// -----------------------------------------------------------------------
// Tier / coverage
// -----------------------------------------------------------------------

/// Coverage = stake × tier_ratio × TIER_COVERAGE_BPS / 10_000. Corrected
/// 2026-07-14 (full source read): V8 keeps ratio and coverage-percentage
/// as TWO separate knobs (`_tierCap`), not one flat multiplier, an admin
/// can lower TIER_COVERAGE_BPS pool-wide without touching the ratios. An
/// earlier draft collapsed these into one number per tier, silently
/// losing that adjustability: fixed here, not carried forward.
/// Tier encoding matches V8: 1=A, 2=B, 3=C (not 0-indexed).
pub const TIER_A_RATIO: i128 = 15;
pub const TIER_B_RATIO: i128 = 10;
pub const TIER_C_RATIO: i128 = 5;
pub const TIER_COVERAGE_BPS: i128 = 10_000; // 100% of ratio, admin-adjustable
pub const TIER_BPS_DENOMINATOR: i128 = 10_000;

// -----------------------------------------------------------------------
// Claim state machine: full 6 states, ported from V8's Claim.status.
// Corrected 2026-07-14 eng review: earlier research had only captured a
// simplified active/forfeited binary; V8's actual enum is richer and a
// 2-state model would silently collapse real transitions.
// -----------------------------------------------------------------------

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ClaimStatus {
    Unused = 0,
    Active = 1,
    Completed = 2,
    Cancelled = 3,
    Reserved = 4,
    PendingTime = 5,
    /// NEW 2026-07-22 (Rule A): 90-day gate met, reservation live, nothing
    /// forfeited yet: waiting on the staker's own `approve_claim` call
    /// within `APPROVE_WINDOW_LEDGERS`. Replaces the old behavior where
    /// meeting the time gate auto-forfeited the stake with no staker
    /// action required.
    AwaitingApproval = 6,
    /// NEW 2026-07-22: terminal state for a reservation that lapsed
    /// without staker action: either Rule A (never approved in time) or
    /// Rule B (approved, then went inactive too long during collection).
    /// Same downstream handling either way: release whatever's still
    /// reserved, stop further action. Distinct from `Cancelled`, which
    /// implies an admin false-positive judgment call and its associated
    /// penalty-lock logic: an expiry is neither party's fault.
    Expired = 7,
}

// -----------------------------------------------------------------------
// StakeRecord: ported from V8's StakeRecord struct, minus yield fields.
// -----------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct StakeRecord {
    /// sha256(beneficiary), plaintext beneficiary address is never stored.
    /// Decided 2026-07-14: sha256 not keccak256 (Soroban-native, consistent
    /// with the Ed25519 oracle-auth decision; no cross-chain hash-matching
    /// requirement exists for this field).
    pub beneficiary_hash: BytesN<32>,
    pub amount: i128,
    /// `YieldIndex` at the moment this stake was created. `withdraw()`
    /// computes the payout as `amount * current_index / yield_index_at_stake`.
    /// This is the snapshot half of that ratio, so a staker only accrues
    /// yield realised AFTER they joined, never retroactively. Fresh stakes
    /// on a pool that has never realised yield snapshot at
    /// `YIELD_INDEX_PRECISION` (the 1.0x starting value), so the ratio is
    /// exactly 1 until the first `extract_yield` call.
    pub yield_index_at_stake: i128,
    pub staked_at_ledger: u32,
    /// Real Unix timestamp at stake time, used ONLY for the absolute
    /// hack_timestamp sanity check in submit_claim ("hack can't predate
    /// the stake"). staked_at_ledger remains the source of truth for all
    /// duration/gate math (time gate, cooldown, vesting, penalty lock).
    pub staked_at_timestamp: u64,
    /// Set by cancel_claim on a false-positive reversal; blocks withdraw
    /// for PENALTY_LOCK_LEDGERS. 0 if never penalized.
    pub penalty_locked_until_ledger: u32,
    pub withdrawn: bool,
    /// Admin can block payout eligibility; does NOT block principal withdrawal.
    pub suspended: bool,
    /// CHANGED 2026-07-22 (eng review, bug 2 fix): was a bare `claim_active:
    /// bool`. A bool alone can't distinguish "re-executing/correcting THIS
    /// SAME claim" from "wallet already has a DIFFERENT claim in flight",
    /// `execute_override` needs that distinction to enforce the one-wallet-
    /// one-claim invariant, which a bool structurally cannot express.
    /// `Some(claim_id)` while any claim is open against this stake (blocks
    /// withdrawal, same as the old bool); `None` once terminal
    /// (Completed/Cancelled/Expired) or never claimed.
    pub active_claim_id: Option<BytesN<32>>,
    /// T3 (2026-08-24): `Some(claim_id)` while a `submit_claim` for this
    /// wallet is sitting in `ClaimStatus::Reserved` (rejected for
    /// `DailyStressCapExceeded`/`Insolvent`, not yet released or expired).
    /// Deliberately separate from `active_claim_id`: a Reserved claim
    /// isn't "active" (it holds no capacity reservation: `total_allocated`
    /// is untouched until release), but the wallet still can't queue a
    /// second one without this guard.
    pub reserved_claim_id: Option<BytesN<32>>,
    /// Pre-audit gate P2 (2026-09-23): set when a claim on this stake is
    /// approved (the stake is forfeited). The address can never stake again
    /// (founder rule). Cleared only by `cancel_claim` undoing that claim.
    pub claim_approved: bool,
}

// -----------------------------------------------------------------------
// v1 (2026-09-22): backers (backer.rs).
// -----------------------------------------------------------------------

/// How long new backer money waits before it counts toward capacity
/// (withdrawal-safety rule 4). Fixed in code, not a setting (founder
/// decision 2026-09-22): it is the guard against deposit spike -> admit
/// claims -> pull out.
pub const BACKER_MATURITY_SECONDS: u64 = 7 * SECONDS_PER_DAY;

#[contracttype]
#[derive(Clone, Debug)]
pub struct BackerRecord {
    /// Matured money: counts toward capacity, can be requested out.
    pub amount: i128,
    /// Deposited but not yet matured: held, does NOT count toward capacity.
    pub pending_amount: i128,
    /// Timestamp at which `pending_amount` matures; 0 when nothing pending.
    pub pending_matures_at: u64,
    /// Open withdrawal request (0 = none). Still counted toward capacity
    /// until completed (rule 2).
    pub withdraw_amount: i128,
    /// Earliest completion timestamp for the open request.
    pub withdraw_ready_at: u64,
}

// -----------------------------------------------------------------------
// Claim: ported from V8's Claim struct.
// -----------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct Claim {
    pub wallet: Address,
    pub tx_hash: BytesN<32>,
    /// Ledger timestamp of the hack event, validated at submit_claim.
    pub hack_timestamp: u64,
    /// Total approved payout.
    pub entitlement: i128,
    /// Already streamed to the beneficiary so far.
    pub streamed: i128,
    /// Stake amount captured at submit_claim, used by cancel_claim to
    /// restore total_staked on a false-positive reversal.
    pub stake: i128,
    pub cooldown_ends_ledger: u32,
    pub vesting_ends_ledger: u32,
    /// CORRECTED 2026-07-14 (second pass, full source read): NOT "fixed at
    /// end of cooldown" as an earlier note claimed. Activation happens
    /// synchronously: either inside submit_claim itself (gate already
    /// met) or inside unlock_pending_claim (gate met later), and
    /// cooldown_ends/vesting_ends are set as FUTURE deadlines starting
    /// from that same activation moment, not preconditions for it. The
    /// snapshot is total_staked read immediately before THIS claim's own
    /// forfeiture decrement, in that same call. Still matters for the
    /// same reason: get the timing wrong and the outflow cap's
    /// anti-manipulation guarantee breaks.
    /// v1 (2026-09-22): now snapshots CAPACITY (`total_staked +
    /// total_backed`), not staker principal alone. Field name kept: struct
    /// field names are part of the stored XDR.
    pub total_staked_snapshot: i128,
    /// Assessed by the oracle at claim time, included in its signed
    /// verdict: never re-derivable/forgeable client-side.
    pub tier: u32,
    pub status: ClaimStatus,
    /// NEW 2026-07-22 (Rule A): set when entering `AwaitingApproval`
    /// (`now + APPROVE_WINDOW_LEDGERS`). Checked by both `approve_claim`
    /// (must call before this) and the permissionless `expire_pending_approval`
    /// (may sweep after this). Unused (0) before that state is reached.
    pub approve_deadline_ledger: u32,
    /// NEW 2026-07-22 (Rule B): set to `cooldown_ends_ledger`, not the
    /// approval ledger: the moment a claim activates (eng review blocker
    /// #2: the mandatory 7-day cooldown must never count as staker
    /// inactivity). Updated to `now` on every successful `claim_stream`.
    /// Checked by the permissionless `expire_stale_claim`. Unused (0)
    /// before activation.
    pub last_collected_ledger: u32,
}

// -----------------------------------------------------------------------
// OverrideRequest: 2-of-2 (oracle + coSigner) override flow.
// -----------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct OverrideRequest {
    pub wallet: Address,
    pub tx_hash: BytesN<32>,
    pub entitlement: i128,
    pub tier: u32,
    /// The actual approving address, not a bare bool, found via
    /// adversarial review (2026-07-14, Solodit "operator retains power
    /// after removal" pattern class): a bool can't tell a CURRENT
    /// admin/coSigner's approval apart from a STALE one left over from
    /// before a set_co_signer/transfer_admin rotation. Readiness is
    /// derived by comparing these against the CURRENT admin/coSigner at
    /// execution time, so a rotation automatically invalidates an old
    /// approval instead of letting it silently carry forward.
    pub owner_approver: Option<Address>,
    pub co_signer_approver: Option<Address>,
}
