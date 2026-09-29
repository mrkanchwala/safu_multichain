//! Typed contract errors: replaces the previous `panic!("SAFU: ...")`
//! string-based failure path. One flat enum for the whole contract (not
//! one per module): Soroban's error surface is a single discriminant
//! space per deployed contract, so splitting by module would only add
//! `From`/enum-of-enums plumbing without changing the on-chain ABI shape.
//! Variant names are module-prefixed-in-spirit via grouping/comments
//! below, not via separate types, so the flat numbering stays a single
//! source of truth. The conversion was rolled out module by module
//! (admin -> stake -> claim).

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PoolError {
    // -- shared / cross-module (1-9) --
    NoStake = 1,
    AlreadyWithdrawn = 2,
    PoolCapNotPositive = 3,

    // -- admin.rs (10-19) --
    AlreadyInitialized = 10,
    OracleEqualsCoSigner = 11,
    CoSignerEqualsAdmin = 12,
    NewAdminEqualsCoSigner = 13,
    PoolCapBelowTotalStaked = 14,
    CoSignerEqualsOracle = 15,

    // -- stake.rs (20-39) --
    StakeNotPositive = 20,
    StakeOutOfRange = 21,
    AlreadyStaked = 22,
    PoolCapExceeded = 23,
    BeneficiaryIsStaker = 24,
    BeneficiaryIsOracle = 25,
    BeneficiaryIsAdmin = 26,
    BeneficiaryIsCoSigner = 27,
    StakeForfeited = 28,
    ClaimActive = 29,
    PenaltyLockActive = 30,
    WrongBeneficiary = 31,
    NoActiveStake = 32,

    // -- claim.rs (40-79) --
    InvalidTier = 40,
    CallerNotOracleOrAdmin = 41,
    EntitlementNotPositive = 42,
    StakeSuspended = 43,
    ClaimAlreadyActiveForStake = 44,
    EntitlementExceedsTierCap = 45,
    HackTimestampInFuture = 46,
    HackPredatesStake = 47,
    ClaimWindowExpired = 48,
    Insolvent = 49,
    DailyStressCapExceeded = 50,
    OracleDailyClaimLimitReached = 51,
    ClaimAlreadyExists = 52,
    NoSuchClaim = 53,
    ClaimNotPending = 54,
    TimeGateNotMet = 55,
    ClaimNotAwaitingApproval = 56,
    ApprovalWindowExpired = 57,
    ApprovalWindowNotExpired = 58,
    ClaimNotActive = 59,
    ClaimFullyStreamed = 60,
    ClaimNotStale = 61,
    CooldownNotPassed = 62,
    NothingVested = 63,
    DailyOutflowCapReached = 64,
    ClaimNotCancellable = 65,
    CallerNotAdminOrCoSigner = 66,
    OverrideParamsMismatch = 67,
    ClaimAlreadyCompleted = 68,
    WalletHasDifferentActiveClaim = 69,
    NoStakeAmountForOverride = 70,
    NoPendingOverride = 71,
    CallerNotAdmin = 72,

    // -- claim.rs / D1 on-chain Ed25519 oracle verification (73-79) --
    //
    // APPENDED at 73, never renumbered into 1-72. Error codes are public
    // ABI (skills.stellar.org/skills/smart-contracts/development.md), a
    // client already matching on `Insolvent = 49` must keep matching on it
    // across this upgrade.
    //
    // Deliberate gap, documented rather than hidden: there is NO error code
    // for "signature did not verify." `env.crypto().ed25519_verify` returns
    // `()` and traps on mismatch (soroban-sdk 27.0.0 `crypto.rs:152`); there
    // is no `try_` variant and a host trap cannot be recovered in-guest. The
    // four codes below exist precisely so every *recoverable* failure
    // reports as a typed error and only a genuine cryptographic mismatch
    // reaches the opaque trap: see `verify_oracle_signature` in claim.rs
    // for the ordering that guarantees this, and note the trap lands before
    // any storage write, so a rejected signature is state-safe.
    /// `deadline` had already passed at submission time.
    SignatureExpired = 73,
    /// Oracle attestation pubkey absent from instance storage. The oracle
    /// claim path is fail-closed until admin sets it; the admin path is
    /// unaffected.
    OraclePubKeyNotSet = 74,
    /// This exact approval payload was revoked by admin before submission.
    ApprovalRevoked = 75,
    /// `deadline` is further out than `MAX_APPROVAL_WINDOW_SECONDS`. Bounds
    /// how long one signed approval can stay live, and is what makes the
    /// revocation TTL provably outlive every legal deadline, see the
    /// const-assert in types.rs.
    SignatureDeadlineTooFar = 76,

    // -- vault.rs / D2 DeFindex vault integration (80-92) --
    //
    // (Named `vault.rs`, not `yield.rs`: `yield` is a reserved Rust keyword
    // and cannot be a module name; see lib.rs. Comment corrected 2026-08-17,
    // 7a audit Finding 6, along with the range: 93-99 is now the pause gate,
    // so D2's block ends at 92.)
    //
    // APPENDED at 80, never renumbered into 1-76. Starts at 80 rather than 77
    // so the 73-79 block stays reserved for the D1 signature layer, error
    // codes are public ABI and a client matching on `Insolvent = 49` must keep
    // matching on it across every upgrade.
    //
    // 80 is the one that matters operationally. Before D2, `total_staked` was
    // identical to the contract's real XLM balance by construction, so every
    // outbound transfer was guaranteed to have funds behind it. Once XLM can
    // sit in the vault the two diverge, and a transfer could fail against a
    // real balance the accounting number knows nothing about. Left unhandled
    // that surfaces as an opaque SAC host trap, the same
    // indistinguishable-failure shape flagged on the scanner's ChainAbuse
    // path. Every outbound transfer therefore pre-checks liquidity and
    // reports this typed code instead, so a staker learns "retry after
    // rebalance" rather than seeing a trap.
    /// Contract's liquid XLM balance is below what this transfer needs, even
    /// after the in-path pull from the vault (v1): no vault, or the vault
    /// refused (e.g. a loss beyond the 5% floor). Principal is not lost: the
    /// admin can `provide_liquidity` with a floor they choose, then retry.
    InsufficientLiquidity = 80,
    /// No vault address configured. Every yield path is fail-closed until
    /// admin calls `set_vault`; the pool simply holds XLM until then.
    VaultNotSet = 81,
    /// Deploying this much would exceed `total_staked * deploy_bps`.
    DeployExceedsCeiling = 82,
    /// Deploying this much would leave liquid XLM below `total_allocated`,
    /// i.e. would put already-reserved claim entitlements into the vault.
    DeployBreachesAllocation = 83,
    /// Refused to repoint `Vault` while shares are still held at the old
    /// one: the accounting would say deployed while the new vault holds
    /// nothing, stranding the position outside contract logic.
    VaultChangeWhileDeployed = 84,
    /// Redeem request exceeds the shares this contract actually holds.
    RedeemExceedsDeployed = 85,
    /// Amount/share argument was zero or negative.
    AmountNotPositive = 86,
    /// `deploy_bps` above `MAX_DEPLOY_BPS`.
    DeployBpsTooHigh = 87,
    /// No treasury address configured for yield extraction.
    TreasuryNotSet = 88,
    /// Nothing is deployed, so there is nothing to redeem or extract.
    NothingDeployed = 89,
    /// Requested yield withdrawal exceeds the excess above staker principal.
    ExceedsYieldBalance = 90,
    /// Vault minted fewer shares than the caller's `min_shares_out` floor.
    MinSharesNotMet = 91,
    /// Redemption returned less XLM than the caller's `min_asset_out` floor.
    MinAmountNotMet = 92,

    // -- shared / pause gate (93-99) --
    //
    // APPENDED at 93, never renumbered into 1-92. Error codes are public ABI.
    //
    // ADDED 2026-08-17 (7a audit, Finding 5). `storage::require_not_paused`
    // was the single surviving `panic!("SAFU: paused")` from the 2026-07-31
    // typed-error conversion: the only bare panic left in production code.
    // It sits on a load-bearing gate (`stake`, `set_beneficiary`, `withdraw`,
    // `submit_claim`, `unlock_pending_claim`, `approve_claim`, `claim_stream`,
    // `approve_override`, `deploy_to_vault` all route through it), so a
    // paused pool reported as an opaque host trap that clients could not
    // match on: and which fuzzing frameworks read as a crash rather than an
    // expected rejection (Veridise Soroban checklist: prefer
    // `panic_with_error!`/typed errors precisely so fuzzers can tell the
    // difference).
    /// Pool is paused. `stake::emergency_exit` and `vault::provide_liquidity`
    /// remain callable by design: they are the pause-time escape path.
    Paused = 93,

    // -- claim.rs / T3 admission-side retry queue (94-97) --
    //
    // APPENDED at 94, never renumbered into 1-93. Error codes are public
    // ABI. Scope: `DailyStressCapExceeded`, `Insolvent`, and (added later
    // the same day) `OracleDailyClaimLimitReached` all queue as
    // `ClaimStatus::Reserved`: the existing-but-previously-unused status
    // value. The shared property is that the claim is genuine and merely
    // un-admittable right now for a reason the claimant does not control.
    // Logic errors (bad tier, expired window, hack predating the stake)
    // still hard-reject. Full reasoning, including why the oracle counter
    // was first excluded and then included, is in `submit_claim`.
    /// A wallet already has a `Reserved` claim pending, one at a time,
    /// same invariant `active_claim_id` already enforces for live claims.
    ClaimAlreadyQueued = 94,
    /// `try_release_queued_claim`/`expire_queued_claim` called on a
    /// `claim_id` that isn't currently `Reserved`.
    NoSuchQueuedClaim = 95,
    /// `try_release_queued_claim` called but the original blocker
    /// (`DailyStressCapExceeded`/`Insolvent`) still applies today: not a
    /// failure, just "still blocked, try again."
    QueueReleaseNotYetEligible = 96,
    /// `expire_queued_claim` called before `hack_timestamp +
    /// CLAIM_WINDOW_SECONDS` has actually passed.
    QueueNotYetExpired = 97,

    // -- settings.rs / v1 adjustable settings (98-104) --
    /// Proposed value is outside the setting's hard bounds.
    SettingOutOfBounds = 98,
    /// No pending change exists for this setting.
    NoPendingSetting = 99,
    /// Co-signer approved a different value than the admin proposed.
    SettingValueMismatch = 100,
    /// Not yet approved, or the 7-day timelock has not passed.
    SettingNotReady = 101,
    /// The pending change was already approved.
    SettingAlreadyApproved = 102,
    /// Would break a cross-setting rule (min stake > max stake, or a rate
    /// band looser than the band below it).
    SettingOrderInvalid = 103,
    /// Raising the pool cap after anyone has staked needs the timelock
    /// (`propose_setting(PoolCap, ..)`); `set_pool_cap` only lowers it then.
    PoolCapRaiseNeedsTimelock = 104,

    // -- backer.rs / v1 backers (105-112) --
    /// No backer record for this address.
    NoBacker = 105,
    /// Nothing is waiting to mature.
    NoPendingBacking = 106,
    /// Pending backing has not reached its maturity time yet.
    BackingNotMature = 107,
    /// A withdrawal request is already open; complete or cancel it first.
    BackerWithdrawalPending = 108,
    /// No open withdrawal request.
    NoBackerWithdrawal = 109,
    /// The notice period has not passed yet.
    BackerNoticeNotPassed = 110,
    /// Rule 3: paying this out would leave open claims without backing.
    /// Not a failure: the request stays open, retry later.
    BackerCapitalNotFree = 111,
    /// Requested more than the backer's matured balance.
    BackerAmountExceedsBalance = 112,

    // -- pre-audit hardening (2026-09-23): stake/claim binding (113-116) --
    /// A claim for this stake is queued. Withdraw or exit once it is
    /// released (then it runs its normal course) or has expired.
    ClaimQueuedForStake = 113,
    /// This address had a claim approved. It can never stake again
    /// (rule set 2026-09-23).
    AddressHasApprovedClaim = 114,
    /// The stake behind this queued claim is no longer the one it was filed
    /// against. The claim can only expire.
    QueuedClaimStakeChanged = 115,
    /// The stake record does not belong to this claim, or was already
    /// forfeited. Nothing to approve.
    ClaimStakeMismatch = 116,

    // -- governance.rs, key-loss-safe governance (117-127) --
    /// Caller is not the admin, co-signer or guardian.
    GovCallerNotRole = 117,
    /// A change of this kind is already pending. Cancel it first.
    GovChangePending = 118,
    /// No pending change of this kind.
    GovNoPendingChange = 119,
    /// The approval names a different change than the one pending.
    GovChangeMismatch = 120,
    /// The role a change replaces cannot propose or approve it.
    GovTargetCannotApprove = 121,
    /// Not approved by two live roles yet, or the wait has not passed.
    GovNotReady = 122,
    /// A recovery proposal cannot be approved; it only waits or is cancelled.
    GovSoloProposal = 123,
    /// Recovery only replaces ANOTHER role.
    GovRecoveryRoleOnly = 124,
    /// Admin, co-signer, guardian and oracle must be four different addresses.
    GovRoleNotDistinct = 125,
    /// This role already voted on this change.
    GovAlreadyApproved = 126,
    /// The role a change replaces cannot cancel it.
    GovCannotCancel = 127,

    // -- r3 yield (128) --
    /// No yield is owed to this staker or backer right now.
    NoYieldOwed = 128,
}
