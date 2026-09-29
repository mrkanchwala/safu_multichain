// SPDX-License-Identifier: Apache-2.0

//! SAFU ProtectionPool: Soroban port of `SAFUPoolV8.sol`.
//!
//! **Tranche 1 (MVP):** staking, points/tier/claim mechanics, payout
//! streaming, on-chain solvency invariant. Yield deployment was deliberately
//! excluded at this stage: see context/knowledge/smartcontract-soroban.md in
//! the research-ops repo for the mechanics map and the eng review that locked
//! that scope boundary (2026-07-14).
//!
//! **Tranche 2 adds** (doc corrected 2026-08-17, 7a audit Finding 6, this
//! header still described T1-only scope after both deliverables had landed):
//! - **D1**: on-chain Ed25519 oracle approval verification (`claim.rs`:
//!   `build_approval_payload` / `verify_oracle_signature`, `set_oracle_pubkey`,
//!   `set_oracle_identity`, `revoke_approval`).
//! - **D2**: DeFindex vault yield deployment (`vault.rs`), which DOES bring a
//!   yield venue into scope, but on a deliberately different policy from V8's
//!   inline 100%-deploy: admin-triggered only, `deploy_bps`-bounded, floored
//!   above `total_allocated`, and never auto-unwound from a user path. See
//!   `vault.rs` and the D2 block in `types.rs` for the full reasoning.
//!
//! See README.md at the repo root for build/test instructions, the full
//! storage model, mechanics ported from V8, deliberate deviations, and
//! current known-open items: kept there so it's visible to anyone
//! reading this repo directly, without needing this doc comment.

#![no_std]
#[cfg(test)]
extern crate std;

mod admin;
mod backer;
mod claim;
mod error;
mod governance;
mod settings;
mod stake;
mod storage;
#[cfg(test)]
mod test;
mod types;
/// D2 (T2), DeFindex vault yield deployment. Named `vault`, not `yield`:
/// `yield` is a reserved Rust keyword and cannot be a module name.
mod vault;

pub use error::PoolError;

/// Test- and fuzz-only surface, gated behind the `testutils` feature so it
/// is never compiled into the deployed WASM.
///
/// Exists for one reason: the fuzz harness has to produce a VALID oracle
/// signature for every `submit_claim` it generates. Fuzzing the 64 signature
/// bytes directly would make essentially every generated input fail
/// `ed25519_verify`: which traps: and libFuzzer treats a trap as a crash,
/// so the run would drown in false findings and hide real ones (Stellar's own
/// fuzzing guidance warns about exactly this, and `ed25519_verify`'s panic is
/// a HOST trap we cannot route through `panic_with_error!`). Fixing the
/// signature to a valid one over the fuzzed payload keeps the rest of the
/// state machine fuzzable, which is what the targets are actually for.
#[cfg(feature = "testutils")]
pub mod testutils {
    pub use crate::claim::build_approval_payload;
}

use soroban_sdk::{contract, contractimpl, Address, BytesN, Env};

pub use crate::governance::{GovChange, GovKind, GovProposal};
pub use crate::settings::{PendingSetting, SettingKey};

use crate::types::{BackerRecord, Claim, StakeRecord};

#[contract]
pub struct ProtectionPool;

#[contractimpl]
impl ProtectionPool {
    /// Deploy-time initialization.
    ///
    /// CHANGED 2026-08-17 (7a audit, Finding 3): was a separate `initialize`
    /// entrypoint. Renamed to `__constructor`, the SDK-recognised constructor
    /// hook, which runs **as part of the deploy invocation itself**.
    ///
    /// Why this mattered: `initialize` authorized the `admin` **argument**
    /// passed to it, and the only thing stopping a second call was the
    /// `AlreadyInitialized` guard. That guard protects against
    /// re-initialization but not against being *first*, between the deploy
    /// transaction and the legitimate init transaction, anyone observing the
    /// chain could call `initialize` with themselves as admin and satisfy
    /// `require_auth` trivially. The T1 testnet deployment used two separate
    /// transactions (README), so the window was real rather than theoretical.
    /// Stellar's own guidance is explicit: prefer `__constructor` because it
    /// "removes the front-running window between deploy and init", and note
    /// that a contract deployed without one can never gain it afterwards,
    /// which is why this had to change before D4's deploy, not after.
    ///
    /// Deployment now aborts atomically if any validation below fails, so a
    /// contract cannot exist in a half-configured state at all.
    #[allow(clippy::too_many_arguments)]
    pub fn __constructor(
        env: Env,
        admin: Address,
        oracle: Address,
        oracle_pubkey: BytesN<32>,
        co_signer: Address,
        guardian: Address,
        xlm_token: Address,
        pool_cap: i128,
    ) -> Result<(), PoolError> {
        admin::initialize(
            &env,
            &admin,
            &oracle,
            &oracle_pubkey,
            &co_signer,
            &guardian,
            &xlm_token,
            pool_cap,
        )
    }

    // -- governance.rs: roles, oracle, vault, treasury, deploy ceiling --
    // Any 2 of admin / co-signer / guardian, 7 days public. One role alone
    // can recover a lost role after 90 days if nobody cancels.

    pub fn propose_change(env: Env, caller: Address, change: GovChange) -> Result<(), PoolError> {
        governance::propose_change(&env, &caller, change)
    }

    pub fn approve_change(env: Env, caller: Address, change: GovChange) -> Result<(), PoolError> {
        governance::approve_change(&env, &caller, change)
    }

    pub fn propose_recovery(env: Env, caller: Address, change: GovChange) -> Result<(), PoolError> {
        governance::propose_recovery(&env, &caller, change)
    }

    /// Returns true once cancelled (standard changes need two roles).
    pub fn cancel_change(env: Env, caller: Address, kind: GovKind) -> Result<bool, PoolError> {
        governance::cancel_change(&env, &caller, kind)
    }

    pub fn execute_change(env: Env, kind: GovKind) -> Result<(), PoolError> {
        governance::execute_change(&env, kind)
    }

    pub fn get_pending_change(env: Env, kind: GovKind) -> Option<GovProposal> {
        governance::get_pending(&env, kind)
    }

    pub fn get_admin(env: Env) -> Address {
        storage::get_admin(&env)
    }

    pub fn get_co_signer(env: Env) -> Address {
        storage::get_co_signer(&env)
    }

    pub fn get_guardian(env: Env) -> Address {
        storage::get_guardian(&env)
    }

    pub fn get_oracle(env: Env) -> Address {
        storage::get_oracle(&env)
    }

    pub fn get_paused_until(env: Env) -> u64 {
        storage::get_paused_until(&env)
    }

    pub fn set_pool_cap(env: Env, new_cap: i128) -> Result<(), PoolError> {
        admin::set_pool_cap(&env, new_cap)
    }

    // -- v1 adjustable settings (settings.rs): propose -> approve -> 7 days -> execute --

    pub fn propose_setting(env: Env, key: SettingKey, value: i128) -> Result<(), PoolError> {
        settings::propose_setting(&env, key, value)
    }

    pub fn approve_setting(env: Env, key: SettingKey, value: i128) -> Result<(), PoolError> {
        settings::approve_setting(&env, key, value)
    }

    pub fn execute_setting(env: Env, key: SettingKey) -> Result<(), PoolError> {
        settings::execute_setting(&env, key)
    }

    pub fn cancel_setting(env: Env, caller: Address, key: SettingKey) -> Result<(), PoolError> {
        settings::cancel_setting(&env, &caller, key)
    }

    pub fn get_setting(env: Env, key: SettingKey) -> i128 {
        settings::get(&env, key)
    }

    pub fn get_pending_setting(env: Env, key: SettingKey) -> Option<PendingSetting> {
        settings::get_pending(&env, key)
    }

    pub fn pause(env: Env) {
        admin::pause(&env);
    }

    pub fn unpause(env: Env) {
        admin::unpause(&env);
    }

    pub fn suspend_stake(env: Env, wallet: Address) -> Result<(), PoolError> {
        admin::suspend_stake(&env, &wallet)
    }

    /// `claim_id` optional: pass the wallet's in-flight claim (if any) so
    /// its Rule A/B deadline clock resets on unsuspend (eng review
    /// blocker #1). `None` if the wallet has no claim needing a reset.
    pub fn unsuspend_stake(
        env: Env,
        wallet: Address,
        claim_id: Option<BytesN<32>>,
    ) -> Result<(), PoolError> {
        admin::unsuspend_stake(&env, &wallet, claim_id)
    }

    // -- stake / withdraw --

    pub fn stake(
        env: Env,
        staker: Address,
        amount: i128,
        beneficiary: Address,
    ) -> Result<(), PoolError> {
        stake::stake(&env, &staker, amount, &beneficiary)
    }

    pub fn withdraw(env: Env, staker: Address, beneficiary: Address) -> Result<(), PoolError> {
        stake::withdraw(&env, &staker, &beneficiary)
    }

    pub fn set_beneficiary(
        env: Env,
        staker: Address,
        new_beneficiary: Address,
    ) -> Result<(), PoolError> {
        stake::set_beneficiary(&env, &staker, &new_beneficiary)
    }

    pub fn emergency_exit(env: Env, staker: Address) -> Result<(), PoolError> {
        stake::emergency_exit(&env, &staker)
    }

    // -- v1 backers (backer.rs): no admin function touches backer money --

    /// Deposit backer money. Counts toward capacity after 7 days.
    pub fn back(env: Env, backer: Address, amount: i128) -> Result<(), PoolError> {
        backer::back(&env, &backer, amount)
    }

    /// Permissionless: start counting a matured deposit toward capacity.
    pub fn mature_backing(env: Env, backer: Address) -> Result<i128, PoolError> {
        backer::mature_backing(&env, &backer)
    }

    /// Start the notice period. Returns the earliest completion timestamp.
    pub fn request_backer_withdrawal(env: Env, backer: Address, amount: i128) -> Result<u64, PoolError> {
        backer::request_withdrawal(&env, &backer, amount)
    }

    pub fn cancel_backer_withdrawal(env: Env, backer: Address) -> Result<(), PoolError> {
        backer::cancel_withdrawal(&env, &backer)
    }

    /// Pay a requested withdrawal to the backer's own address, once the
    /// notice has passed and only from capital open claims don't need.
    pub fn complete_backer_withdrawal(env: Env, backer: Address) -> Result<i128, PoolError> {
        backer::complete_withdrawal(&env, &backer)
    }

    pub fn get_backer(env: Env, backer: Address) -> Option<BackerRecord> {
        storage::get_backer(&env, &backer)
    }

    pub fn get_total_backed(env: Env) -> i128 {
        storage::get_total_backed(&env)
    }

    pub fn get_total_backed_pending(env: Env) -> i128 {
        storage::get_total_backed_pending(&env)
    }

    /// Staker principal + matured backer money: what claims are sized against.
    pub fn get_capacity(env: Env) -> i128 {
        storage::get_capacity(&env)
    }

    // -- claims --

    /// CHANGED T2/D1: `deadline` + `signature` are now required arguments.
    /// When `caller` is the oracle the contract verifies an Ed25519
    /// signature over the verdict on-chain; when `caller` is the admin
    /// (manual fallback) both are ignored, exactly as in V8. See
    /// `claim::submit_claim` for the full auth-model rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_claim(
        env: Env,
        caller: Address,
        wallet: Address,
        tx_hash: BytesN<32>,
        entitlement: i128,
        tier: u32,
        hack_timestamp: u64,
        deadline: u64,
        signature: BytesN<64>,
    ) -> Result<BytesN<32>, PoolError> {
        claim::submit_claim(
            &env,
            &caller,
            &wallet,
            &tx_hash,
            entitlement,
            tier,
            hack_timestamp,
            deadline,
            &signature,
        )
    }

    /// T2/D1: admin-only. Cancels a signed-but-not-yet-submitted oracle
    /// approval. Takes the full approval parameters rather than a
    /// precomputed hash (V8's shape); see `claim::revoke_approval` for why.
    #[allow(clippy::too_many_arguments)]
    pub fn revoke_approval(
        env: Env,
        caller: Address,
        wallet: Address,
        tx_hash: BytesN<32>,
        entitlement: i128,
        tier: u32,
        hack_timestamp: u64,
        deadline: u64,
    ) -> Result<(), PoolError> {
        claim::revoke_approval(
            &env,
            &caller,
            &wallet,
            &tx_hash,
            entitlement,
            tier,
            hack_timestamp,
            deadline,
        )
    }

    pub fn unlock_pending_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::unlock_pending_claim(&env, &claim_id)
    }

    /// T3 (2026-08-24), permissionless. Re-checks a queued claim's original
    /// blocker (`DailyStressCapExceeded`/`Insolvent`) against current state;
    /// admits it for real if that now passes, otherwise returns
    /// `QueueReleaseNotYetEligible` and it stays `Reserved`.
    pub fn try_release_queued_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::try_release_queued_claim(&env, &claim_id)
    }

    /// T3 (2026-08-24), permissionless. Sweeps a queued claim whose claim
    /// window ran out before capacity ever freed.
    pub fn expire_queued_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::expire_queued_claim(&env, &claim_id)
    }

    /// NEW 2026-07-22 (Rule A), staker-authorized. Burns the wallet's
    /// entire lifetime points balance, forfeits the stake, starts
    /// cooldown/vesting. Must be called within `APPROVE_WINDOW_LEDGERS` of
    /// the claim entering `AwaitingApproval`.
    pub fn approve_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::approve_claim(&env, &claim_id)
    }

    /// NEW 2026-07-22 (Rule A sweep), permissionless, mirrors
    /// `unlock_pending_claim`. Releases the reservation back to the pool
    /// if the staker never approved within the window.
    pub fn expire_pending_approval(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::expire_pending_approval(&env, &claim_id)
    }

    /// NEW 2026-07-22 (Rule B sweep), permissionless. Releases whatever's
    /// left uncollected if the staker goes `COLLECTION_INACTIVITY_LEDGERS`
    /// with zero `claim_stream` activity.
    pub fn expire_stale_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::expire_stale_claim(&env, &claim_id)
    }

    pub fn claim_stream(
        env: Env,
        claim_id: BytesN<32>,
        beneficiary: Address,
    ) -> Result<i128, PoolError> {
        claim::claim_stream(&env, &claim_id, &beneficiary)
    }

    pub fn cancel_claim(env: Env, claim_id: BytesN<32>) -> Result<(), PoolError> {
        claim::cancel_claim(&env, &claim_id)
    }

    pub fn approve_override(
        env: Env,
        caller: Address,
        wallet: Address,
        tx_hash: BytesN<32>,
        entitlement: i128,
        tier: u32,
    ) -> Result<(), PoolError> {
        claim::approve_override(&env, &caller, &wallet, &tx_hash, entitlement, tier)
    }

    pub fn cancel_pending_override(
        env: Env,
        caller: Address,
        wallet: Address,
        tx_hash: BytesN<32>,
    ) -> Result<(), PoolError> {
        claim::cancel_pending_override(&env, &caller, &wallet, &tx_hash)
    }

    // -- views --
    // V8 parity gap closed 2026-07-14, corrected same day: the original
    // "closed" pass only ported raw storage getters (stakeOf-equivalent),
    // missing that V8's isEligible/pointsOf/isClaimEligible are COMPUTED
    // views, not storage reads: found via the same full-source-read that
    // caught the set_co_signer gap. pointsOf in particular returns LIVE
    // computed points for a still-active staker, not the banked balance
    // (which is only meaningful post-withdrawal), get_points_balance
    // below was returning the wrong number for anyone still staked until
    // this fix.

    pub fn get_stake(env: Env, wallet: Address) -> Option<StakeRecord> {
        storage::get_stake(&env, &wallet)
    }

    pub fn get_claim(env: Env, claim_id: BytesN<32>) -> Option<Claim> {
        storage::get_claim(&env, &claim_id)
    }

    /// V8 `pointsOf`: live-computed if still staked and not withdrawn,
    /// else the banked balance. NOT a raw storage read.
    pub fn get_points_balance(env: Env, wallet: Address) -> i128 {
        if let Some(record) = storage::get_stake(&env, &wallet) {
            if record.amount > 0 && !record.withdrawn {
                return stake::compute_points_for_record(&env, &record);
            }
        }
        storage::get_points_balance(&env, &wallet)
    }

    /// V8 `isEligible`: has an active, non-withdrawn, non-suspended
    /// stake. Does NOT check the time gate, see is_claim_eligible.
    pub fn is_eligible(env: Env, wallet: Address) -> bool {
        match storage::get_stake(&env, &wallet) {
            Some(r) => r.amount > 0 && !r.withdrawn && !r.suspended,
            None => false,
        }
    }

    /// V8 `isClaimEligible`: is_eligible AND the 90-day time gate has
    /// passed.
    pub fn is_claim_eligible(env: Env, wallet: Address) -> bool {
        match storage::get_stake(&env, &wallet) {
            Some(r) => {
                r.amount > 0
                    && !r.withdrawn
                    && !r.suspended
                    && env.ledger().sequence().saturating_sub(r.staked_at_ledger)
                        >= crate::types::TIME_GATE_LEDGERS
            }
            None => false,
        }
    }

    // -- D2 yield deployment (all admin-authorized) --
    //
    // v1 (2026-09-24): user calls also rebalance, inside the pool
    // (vault.rs rule 3): payments pull when cash is short, stake/back push
    // idle cash. The functions below stay for the admin (first deposit,
    // manual rescue) and for anyone who wants to rebalance early.

    /// Supply liquid XLM into the vault. Returns shares gained.
    /// `min_shares_out` is the caller's floor against an adverse share
    /// price: quote it off-chain via the vault's
    /// `get_asset_amounts_per_shares` first.
    pub fn deploy_to_vault(
        env: Env,
        amount: i128,
        min_shares_out: i128,
    ) -> Result<i128, PoolError> {
        vault::deploy_to_vault(&env, amount, min_shares_out)
    }

    /// T3 (2026-08-24), permissionless sibling of `deploy_to_vault`. Puts
    /// idle liquidity (above what's reserved for live claims) to work
    /// automatically, up to the existing `deploy_bps` ceiling. No caller
    /// input: the contract computes the amount and its own slippage
    /// floor. Requires at least one prior manual `deploy_to_vault` call
    /// (needs a reference rate); returns 0 if nothing's idle or no room
    /// remains under the ceiling.
    pub fn auto_deploy_liquidity(env: Env) -> Result<i128, PoolError> {
        vault::auto_deploy_liquidity(&env)
    }

    /// Redeem shares so the XLM sits liquid in the contract, ready to fund
    /// payouts and withdrawals. Nothing leaves the pool. Works while
    /// paused, deliberately: `emergency_exit` depends on it.
    pub fn provide_liquidity(
        env: Env,
        shares: i128,
        min_asset_out: i128,
    ) -> Result<i128, PoolError> {
        vault::provide_liquidity(&env, shares, min_asset_out)
    }

    /// T3 (2026-08-24), permissionless sibling of `provide_liquidity`.
    /// Pulls back only enough to cover `total_allocated` if liquid balance
    /// is short of it. No caller input. Works while paused, same reasoning
    /// as `provide_liquidity`. Returns 0 if nothing's short.
    pub fn ensure_liquidity(env: Env) -> Result<i128, PoolError> {
        vault::ensure_liquidity(&env)
    }

    /// Redeem a tranche and send only the excess above proportional
    /// principal to treasury. Returns the yield amount (0 on a loss).
    pub fn extract_yield(env: Env, shares: i128, min_asset_out: i128) -> Result<i128, PoolError> {
        vault::extract_yield(&env, shares, min_asset_out)
    }

    /// Send already-liquid excess above staker principal to treasury.
    pub fn withdraw_yield(env: Env, amount: i128) -> Result<(), PoolError> {
        vault::withdraw_yield(&env, amount)
    }

    // -- D2 views --

    /// Real liquid XLM held by the contract, distinct from
    /// `get_total_staked`, which counts XLM sitting in the vault too.
    pub fn get_liquid_balance(env: Env) -> i128 {
        vault::liquid_balance(&env)
    }

    /// XLM in the vault at ORIGINAL deposit value, never marked to market.
    pub fn get_total_deployed_asset(env: Env) -> i128 {
        storage::get_total_deployed_asset(&env)
    }

    pub fn get_total_deployed_shares(env: Env) -> i128 {
        storage::get_total_deployed_shares(&env)
    }

    /// The protocol's own realised, unwithdrawn yield share, "the
    /// protocol's own money", withdrawable any time via
    /// `withdraw_yield`. CHANGED 2026-09-18: now reads the explicit
    /// `ProtocolYieldBalance` counter, not the removed residual formula
    /// (documented bug history: see vault.rs).
    pub fn get_yield_balance(env: Env) -> i128 {
        storage::get_protocol_yield_balance(&env)
    }

    /// Aave-style growing multiplier. 1.0x is `YIELD_INDEX_PRECISION`; grows
    /// only via `extract_yield`'s staker-share split. Exposed mainly for
    /// audit/observability: callers wanting a staker's actual withdrawable
    /// balance should call `get_withdrawable_amount` instead of re-deriving
    /// the ratio themselves.
    pub fn get_yield_index(env: Env) -> i128 {
        storage::get_yield_index(&env)
    }

    /// A staker's current withdrawable value, principal plus their
    /// proportional share of yield realised since they staked. 0 for no
    /// stake, a withdrawn stake, or a forfeited (claimed) one.
    pub fn get_withdrawable_amount(env: Env, staker: Address) -> i128 {
        vault::withdrawable_amount(&env, &staker)
    }

    pub fn get_total_extracted_yield(env: Env) -> i128 {
        storage::get_total_extracted_yield(&env)
    }

    pub fn get_deploy_bps(env: Env) -> i128 {
        storage::get_deploy_bps(&env)
    }

    pub fn get_vault(env: Env) -> Option<Address> {
        storage::get_vault(&env)
    }

    /// Current deployed fraction in bps. May read above `get_deploy_bps`
    /// after a large withdrawal: that is drift, not a breach; see vault.rs.
    pub fn get_deployment_ratio_bps(env: Env) -> i128 {
        vault::deployment_ratio_bps(&env)
    }

    pub fn get_total_staked(env: Env) -> i128 {
        storage::get_total_staked(&env)
    }

    pub fn get_total_allocated(env: Env) -> i128 {
        storage::get_total_allocated(&env)
    }

    pub fn get_total_stakers(env: Env) -> u32 {
        storage::get_total_stakers(&env)
    }

    pub fn is_paused(env: Env) -> bool {
        storage::is_paused(&env)
    }
}
