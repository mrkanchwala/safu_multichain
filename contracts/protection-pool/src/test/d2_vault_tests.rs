//! T2/D2: DeFindex vault yield deployment.
//!
//! Split out rather than folded into `stake_tests`/`solvency_tests` because
//! these test the liquid-vs-deployed accounting layer itself, not pool
//! mechanics. The pool mechanics tests continue to pass unchanged, which is
//! itself the headline assertion: D2 required NO change to the solvency
//! gate, `stress_cap`, or `dynamic_outflow_bps`.
//!
//! **What the mock vault does and does not prove.** `MockVault` reproduces
//! the three functions of the real DeFindex ABI that this contract calls,
//! with a settable share/XLM rate so appreciation and loss are both
//! testable.
//!
//! It also reproduces the real vault's AUTHORIZATION shape, which is the one
//! thing an earlier draft of this file recorded as un-knowable without
//! testnet. It is now known: on 2026-08-14 a `deposit` was simulated against
//! the live testnet vault over Soroban RPC and the auth tree it returned
//! decoded to `deposit(vault, ..) -> transfer(token, [from, vault, amount])`.
//! So DeFindex's `deposit` calls `from.require_auth()` itself, and it pulls
//! via `transfer`, not `transfer_from`. The mock therefore calls
//! `from.require_auth()` in both `deposit` and `withdraw`, WITHOUT that, the
//! mock would be laxer than the contract it stands in for and would let a
//! missing `authorize_as_current_contract(..)` reach testnet unnoticed.
//!
//! Note `env.mock_all_auths()` does NOT paper over any of this: it does not
//! cover invoker-CONTRACT auth, which is exactly why this suite caught the
//! missing authorization rather than deferring it to D4.
//!
//! `withdraw` was verified the same way, against a real testnet position,
//! since the vault reverts at its own share-balance check before reaching any
//! sub-invocation otherwise. It returned a ONE-level tree with no
//! sub-invocations, so the mock needs no nested authorization on that side
//! either. Both halves of this integration's auth surface are now measured
//! against the deployed contract rather than inferred.
//!
//! The mock mints shares 1:1 BY DEFAULT, where the real vault does not
//! (10,000,000 stroops bought 5,996 dfTokens). That default keeps the
//! accounting assertions readable, and it was safe for D2 because
//! `deploy_to_vault` measures shares by balance delta rather than assuming a
//! rate.
//!
//! **T3 (2026-08-24) made that no longer sufficient on its own.** T3 added
//! share-price arithmetic (`expected_shares`, and a `min_shares_out` slippage
//! floor computed from it) which a 1:1 rate collapses to a no-op, a
//! cargo-mutants run showed every mutation of that formula surviving. Use
//! `set_deposit_rate_bps` to mint non-1:1 when exercising anything that reads
//! the rate; the 1:1 default remains for every test that does not.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec};

use crate::error::PoolError;
use crate::test::common::*;
use crate::types::{COOLDOWN_LEDGERS, LEDGERS_PER_DAY, YIELD_INDEX_PRECISION};

// -----------------------------------------------------------------------
// Mock vault: mirrors the real DeFindex vault's `deposit`/`withdraw`/
// `balance` signatures, verified on-chain 2026-08-14 via
// `stellar contract info interface`.
//
// Shares are minted 1:1 with deposited XLM. Redemption pays
// `shares * rate_bps / 10_000`, so `rate_bps` above 10_000 simulates yield
// and below simulates a Blend loss. The vault must be funded separately to
// pay out above par: same as reality, where the gain comes from Blend.
// -----------------------------------------------------------------------

#[contracttype]
pub enum MockKey {
    Token,
    RateBps,
    /// T3 (2026-08-24): shares minted per unit deposited, in bps. Defaults to
    /// 10_000 (1:1) so every pre-existing test is unaffected.
    ///
    /// Added because the 1:1 default made T3's new share-price arithmetic
    /// structurally untestable: with `deployed_shares == deployed_asset`,
    /// `expected_shares = amount * shares / xlm` collapses to `amount`, and a
    /// cargo-mutants run showed every mutation of that formula and of
    /// `min_shares_out` surviving. The module header above calls the 1:1
    /// simplification "safe because deploy_to_vault measures shares by
    /// balance delta": true before T3, but T3 added a slippage floor COMPUTED
    /// from the rate, which a 1:1 mock can never exercise. Reality is not 1:1
    /// either: 10,000,000 stroops bought 5,996 dfTokens on testnet.
    DepositRateBps,
    Shares(Address),
    /// v1 (2026-09-24): failure switches for the in-path pull/push tests.
    FailWithdraw,
    FailDeposit,
    /// Largest share count `withdraw` will serve (simulates Blend running dry).
    MaxWithdrawShares,
    /// Whether `withdraw` enforces `min_amounts_out` (default true, like the
    /// real vault). Off only to reach the pool's own `MinAmountNotMet` guard.
    NoMinCheck,
}

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn init(env: Env, token: Address) {
        env.storage().instance().set(&MockKey::Token, &token);
        env.storage().instance().set(&MockKey::RateBps, &10_000i128);
    }

    /// >10_000 = the position gained; <10_000 = it lost.
    pub fn set_rate_bps(env: Env, bps: i128) {
        env.storage().instance().set(&MockKey::RateBps, &bps);
    }

    /// T3: shares minted per unit deposited, in bps. <10_000 mints fewer
    /// shares than XLM in (the real vault's behaviour).
    pub fn set_deposit_rate_bps(env: Env, bps: i128) {
        env.storage().instance().set(&MockKey::DepositRateBps, &bps);
    }

    pub fn set_fail_withdraw(env: Env, fail: bool) {
        env.storage().instance().set(&MockKey::FailWithdraw, &fail);
    }

    pub fn set_fail_deposit(env: Env, fail: bool) {
        env.storage().instance().set(&MockKey::FailDeposit, &fail);
    }

    pub fn set_max_withdraw_shares(env: Env, max: i128) {
        env.storage().instance().set(&MockKey::MaxWithdrawShares, &max);
    }

    /// Turn off the vault-side floor so a test reaches the pool's own guard.
    pub fn disable_min_check(env: Env) {
        env.storage().instance().set(&MockKey::NoMinCheck, &true);
    }

    pub fn deposit(
        env: Env,
        amounts_desired: Vec<i128>,
        _amounts_min: Vec<i128>,
        from: Address,
        _invest: bool,
    ) -> i128 {
        // Verified against the live testnet vault, see the module header.
        from.require_auth();
        if env.storage().instance().get(&MockKey::FailDeposit).unwrap_or(false) {
            panic!("mock vault: deposit failure");
        }
        let amount = amounts_desired.get(0).unwrap();
        let token: Address = env.storage().instance().get(&MockKey::Token).unwrap();
        TokenClient::new(&env, &token).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );
        let cur: i128 = env
            .storage()
            .instance()
            .get(&MockKey::Shares(from.clone()))
            .unwrap_or(0);
        // T3: mint at DepositRateBps (default 1:1) rather than unconditionally
        // 1:1, so the contract's own share-price arithmetic is exercisable.
        let deposit_rate: i128 = env
            .storage()
            .instance()
            .get(&MockKey::DepositRateBps)
            .unwrap_or(10_000);
        let minted = amount * deposit_rate / 10_000;
        env.storage()
            .instance()
            .set(&MockKey::Shares(from), &(cur + minted));
        minted
    }

    pub fn withdraw(
        env: Env,
        withdraw_shares: i128,
        min_amounts_out: Vec<i128>,
        from: Address,
    ) -> i128 {
        from.require_auth();
        if env.storage().instance().get(&MockKey::FailWithdraw).unwrap_or(false) {
            panic!("mock vault: withdraw failure");
        }
        if let Some(max) = env.storage().instance().get::<_, i128>(&MockKey::MaxWithdrawShares) {
            if withdraw_shares > max {
                panic!("mock vault: not enough liquidity in the strategy");
            }
        }
        let token: Address = env.storage().instance().get(&MockKey::Token).unwrap();
        let rate: i128 = env.storage().instance().get(&MockKey::RateBps).unwrap();
        let cur: i128 = env
            .storage()
            .instance()
            .get(&MockKey::Shares(from.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&MockKey::Shares(from.clone()), &(cur - withdraw_shares));
        let out = withdraw_shares * rate / 10_000;
        // v1: the real DeFindex vault enforces `min_amounts_out` itself; the
        // in-path pull relies on that, so the mock must too.
        let check = !env.storage().instance().get(&MockKey::NoMinCheck).unwrap_or(false);
        if check && out < min_amounts_out.get(0).unwrap_or(0) {
            panic!("mock vault: below min_amounts_out");
        }
        TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &from,
            &out,
        );
        out
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&MockKey::Shares(id))
            .unwrap_or(0)
    }

    /// r3: mirrors the real vault's view (signature verified over RPC):
    /// the asset value of `vault_shares` at the current redemption rate.
    pub fn get_asset_amounts_per_shares(env: Env, vault_shares: i128) -> Vec<i128> {
        let rate: i128 = env.storage().instance().get(&MockKey::RateBps).unwrap();
        let mut out = Vec::new(&env);
        out.push_back(vault_shares * rate / 10_000);
        out
    }
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Registers a mock vault, wires it into the pool, and opens deployment to
/// `deploy_bps`. Returns (vault address, mock client).
pub(crate) fn with_vault<'a>(env: &'a Env, s: &Setup<'a>, deploy_bps: i128) -> (Address, MockVaultClient<'a>) {
    let vault_id = env.register(MockVault, ());
    let mock = MockVaultClient::new(env, &vault_id);
    mock.init(&s.token_id);
    gov_apply(&env, &s, crate::GovChange::Vault(vault_id.clone()));
    gov_apply(&env, &s, crate::GovChange::DeployBps(deploy_bps));
    (vault_id, mock)
}

/// The invariant every test asserts: the pool always controls at least what
/// it owes stakers, counting deployed principal at original value.
pub(crate) fn assert_invariant(s: &Setup<'_>) {
    let liquid = s.client.get_liquid_balance();
    let deployed = s.client.get_total_deployed_asset();
    let staked = s.client.get_total_staked();
    assert!(
        liquid + deployed >= staked,
        "SOLVENCY INVARIANT VIOLATED: liquid {} + deployed {} < staked {}",
        liquid,
        deployed,
        staked
    );
}

// -----------------------------------------------------------------------
// Configuration + fail-closed defaults
// -----------------------------------------------------------------------

#[test]
fn yield_layer_is_inert_on_a_fresh_deploy() {
    let env = new_env();
    let s = setup(&env);
    // The whole point of not touching `initialize`'s signature: a fresh
    // contract behaves exactly as it did in Tranche 1.
    assert_eq!(s.client.get_deploy_bps(), 0);
    assert_eq!(s.client.get_vault(), None);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
    assert_eq!(s.client.get_total_deployed_shares(), 0);
}

#[test]
fn deploy_without_a_vault_fails_closed() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    gov_apply(&env, &s, crate::GovChange::DeployBps(5_000));
    assert_eq!(
        s.client.try_deploy_to_vault(&STROOPS_PER_UNIT, &0),
        Err(Ok(PoolError::VaultNotSet))
    );
}

#[test]
fn deploy_bps_is_hard_capped() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(
        s.client.try_propose_change(&s.admin, &crate::GovChange::DeployBps(8_001)),
        Err(Ok(PoolError::DeployBpsTooHigh))
    );
    gov_apply(&env, &s, crate::GovChange::DeployBps(8_000)); // the cap itself is allowed
    assert_eq!(s.client.get_deploy_bps(), 8_000);
}

#[test]
fn set_vault_is_refused_while_shares_are_held() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (_v, _m) = with_vault(&env, &s, 5_000);
    s.client.deploy_to_vault(&(MID_STAKE / 4), &0);

    let other = env.register(MockVault, ());
    assert_eq!(
        s.client.try_propose_change(&s.admin, &crate::GovChange::Vault(other.clone())),
        Err(Ok(PoolError::VaultChangeWhileDeployed))
    );

    // Redeem everything, and it becomes settable again.
    let shares = s.client.get_total_deployed_shares();
    s.client.provide_liquidity(&shares, &0);
    gov_apply(&env, &s, crate::GovChange::Vault(other.clone()));
    assert_eq!(s.client.get_vault(), Some(other));
}

// -----------------------------------------------------------------------
// Deployment bounds
// -----------------------------------------------------------------------

#[test]
fn deploy_respects_the_bps_ceiling() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s); // MID_STAKE staked
    with_vault(&env, &s, 5_000); // 50%

    let half = MID_STAKE / 2;
    assert_eq!(
        s.client.try_deploy_to_vault(&(half + 1), &0),
        Err(Ok(PoolError::DeployExceedsCeiling))
    );
    s.client.deploy_to_vault(&half, &0);
    assert_eq!(s.client.get_total_deployed_asset(), half);
    assert_invariant(&s);
}

#[test]
fn deploy_cannot_touch_reserved_entitlements() {
    let env = new_env();
    let s = setup(&env);
    let (wallet, _b) = staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);

    // Put a claim in flight so total_allocated is non-zero.
    //
    // MID_STAKE / 4, not / 2: at zero utilization `claim::stress_cap` admits
    // 2,500 bps of `total_staked` in a single day, so MID_STAKE / 2 is
    // refused by the admission-side cap (`DailyStressCapExceeded`) long
    // before this test's actual subject, the deployment floor, is reached.
    // A quarter sits exactly at that cap, which is admitted (the check is
    // `>`), and still leaves the arithmetic below unambiguous.
    advance_days(&env, 91);
    let entitlement = MID_STAKE / 4;
    submit_claim_signed(
        &env,
        &s,
        &s.oracle.clone(),
        &wallet,
        &tx_hash(&env, 1),
        &entitlement,
        &1u32,
        &now_ts(&env),
    );
    assert_eq!(s.client.get_total_allocated(), entitlement);

    // 80% of MID_STAKE is deployable by the ceiling, but the floor refuses
    // anything that would leave liquid below the reserved entitlement.
    let liquid = s.client.get_liquid_balance();
    let too_much = liquid - entitlement + 1;
    assert_eq!(
        s.client.try_deploy_to_vault(&too_much, &0),
        Err(Ok(PoolError::DeployBreachesAllocation))
    );
    s.client.deploy_to_vault(&(liquid - entitlement), &0);
    assert_invariant(&s);
}

#[test]
fn deploy_enforces_the_min_shares_floor() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 5_000);

    let amount = MID_STAKE / 4;
    // Mock mints 1:1, so demanding more shares than XLM must fail.
    assert_eq!(
        s.client.try_deploy_to_vault(&amount, &(amount + 1)),
        Err(Ok(PoolError::MinSharesNotMet))
    );
    s.client.deploy_to_vault(&amount, &amount);
}

#[test]
fn deploy_rejects_non_positive_amounts() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 5_000);
    assert_eq!(
        s.client.try_deploy_to_vault(&0, &0),
        Err(Ok(PoolError::AmountNotPositive))
    );
    assert_eq!(
        s.client.try_deploy_to_vault(&-1, &0),
        Err(Ok(PoolError::AmountNotPositive))
    );
}

// -----------------------------------------------------------------------
// The liquidity checks. v1 (2026-09-24): a short payment pulls from the
// vault itself; the typed error remains for when the vault refuses.
// -----------------------------------------------------------------------

#[test]
fn withdraw_pulls_from_the_vault_when_liquidity_is_short() {
    let env = new_env();
    let s = setup(&env);
    let (staker, beneficiary) = staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);

    // 20% liquid, 80% in the vault: the withdrawal redeems what it needs.
    s.client.withdraw(&staker, &beneficiary);
    assert_eq!(s.client.get_total_staked(), 0);
    assert_eq!(s.client.get_total_deployed_asset(), 0);
    assert_invariant(&s);
}

#[test]
fn withdraw_reports_a_typed_error_when_the_vault_refuses() {
    let env = new_env();
    let s = setup(&env);
    let (staker, beneficiary) = staked_wallet(&env, &s);
    let (_v, mock) = with_vault(&env, &s, 8_000);
    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);

    // Principal is not lost: it is in the vault, which refuses right now.
    mock.set_fail_withdraw(&true);
    assert_eq!(
        s.client.try_withdraw(&staker, &beneficiary),
        Err(Ok(PoolError::InsufficientLiquidity))
    );
    // The failed pull left nothing behind.
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    assert_invariant(&s);

    mock.set_fail_withdraw(&false);
    s.client.withdraw(&staker, &beneficiary);
    assert_eq!(s.client.get_total_staked(), 0);
    assert_invariant(&s);
}

#[test]
fn emergency_exit_pulls_from_the_vault_while_paused() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _b) = staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);

    s.client.pause();
    s.client.emergency_exit(&staker);
    assert_eq!(s.client.get_total_staked(), 0);
    assert_invariant(&s);
}

#[test]
fn emergency_exit_reports_a_typed_error_when_the_vault_refuses() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _b) = staked_wallet(&env, &s);
    let (_v, mock) = with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);

    s.client.pause();
    mock.set_fail_withdraw(&true);
    assert_eq!(
        s.client.try_emergency_exit(&staker),
        Err(Ok(PoolError::InsufficientLiquidity))
    );
}

#[test]
fn provide_liquidity_works_while_paused_so_emergency_exit_can_be_funded() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _b) = staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);

    s.client.pause();
    assert!(s.client.is_paused());

    // This is the deliberate deviation from V8, which marks its equivalent
    // `whenNotPaused`. Without it the pause-time escape hatch below could
    // never be funded.
    let shares = s.client.get_total_deployed_shares();
    s.client.provide_liquidity(&shares, &0);

    s.client.emergency_exit(&staker);
    assert_eq!(s.client.get_total_staked(), 0);
    assert_invariant(&s);
}

#[test]
fn deploy_to_vault_is_blocked_while_paused() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 5_000);
    s.client.pause();
    // Redemption works while paused; DEPLOYMENT does not. Money may always
    // come home during an incident, never leave.
    assert_eq!(
        s.client.try_deploy_to_vault(&(MID_STAKE / 4), &0),
        Err(Ok(PoolError::Paused))
    );
}

#[test]
fn claim_stream_pulls_from_the_vault_when_liquidity_is_short() {
    let env = new_env();
    let s = setup(&env);
    let (wallet, beneficiary) = staked_wallet(&env, &s);
    let (other, other_beneficiary) = staked_wallet(&env, &s);
    let (_v, mock) = with_vault(&env, &s, 8_000);

    // Reaching this check needs more than a large deployment, and the reason
    // is worth recording. A single `claim_stream` call is bounded by
    // `dynamic_outflow_bps`, at most 500 bps of `cap_base`, while
    // `MAX_DEPLOY_BPS` guarantees at least 2,000 bps of the pool stays
    // liquid. 2,000 > 500, so deployment ALONE can never starve a stream,
    // the two bounds are ordered such that the liquidity check is
    // unreachable that way.
    //
    // What does reach it is drift. `activate_claim` freezes
    // `total_staked_snapshot` and forfeits the claimant's principal, and
    // `cap_base` is `max(total_staked_now, snapshot)`: so the outflow cap
    // keeps sizing itself against the ORIGINAL pool while the pool itself
    // shrinks underneath it. A second staker leaving is enough. That is a
    // real sequence, not a contrived one, and it is exactly the no-window
    // withdrawal path D2's design flagged.
    //
    // Deploy half, not the 8,000-bps ceiling, so `other` can still exit.
    let deployed = MID_STAKE;
    s.client.deploy_to_vault(&deployed, &0);

    advance_days(&env, 91);
    // 2,500 bps of the two-staker pool, the stress cap admits exactly this.
    let entitlement = MID_STAKE / 2;
    let claim_id = submit_claim_signed(
        &env,
        &s,
        &s.oracle.clone(),
        &wallet,
        &tx_hash(&env, 2),
        &entitlement,
        &1u32,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id);

    // `other` exits and takes every remaining liquid stroop with it. The
    // principal backing the claim is not lost, it is in the vault, but the
    // contract now physically holds nothing.
    s.client.withdraw(&other, &other_beneficiary);
    assert_eq!(s.client.get_liquid_balance(), 0);

    advance_ledgers(&env, COOLDOWN_LEDGERS + 10 * LEDGERS_PER_DAY);

    // The vault refusing is the only way the stream fails now.
    mock.set_fail_withdraw(&true);
    assert_eq!(
        s.client.try_claim_stream(&claim_id, &beneficiary),
        Err(Ok(PoolError::InsufficientLiquidity))
    );

    // Vault back: the stream pulls what it needs itself, no admin call.
    mock.set_fail_withdraw(&false);
    let paid = s.client.claim_stream(&claim_id, &beneficiary);
    assert!(paid > 0);
    assert_invariant(&s);
}

// -----------------------------------------------------------------------
// Redemption, yield, and loss
// -----------------------------------------------------------------------

#[test]
fn redemption_is_proportional_and_multiplies_before_dividing() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);

    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);
    let shares = s.client.get_total_deployed_shares();

    // Redeem a quarter; three quarters of principal must remain booked.
    s.client.provide_liquidity(&(shares / 4), &0);
    assert_eq!(s.client.get_total_deployed_shares(), shares - shares / 4);
    assert_eq!(s.client.get_total_deployed_asset(), deployed - deployed / 4);
    assert_invariant(&s);
}

#[test]
fn redeem_bounds_are_enforced() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 5_000);

    assert_eq!(
        s.client.try_provide_liquidity(&1, &0),
        Err(Ok(PoolError::NothingDeployed))
    );

    s.client.deploy_to_vault(&(MID_STAKE / 4), &0);
    let shares = s.client.get_total_deployed_shares();
    assert_eq!(
        s.client.try_provide_liquidity(&(shares + 1), &0),
        Err(Ok(PoolError::RedeemExceedsDeployed))
    );
    assert_eq!(
        s.client.try_provide_liquidity(&0, &0),
        Err(Ok(PoolError::AmountNotPositive))
    );
}

#[test]
fn harvest_credits_only_the_growth_above_book_value() {
    // Rewritten 2026-09-18 for the yield split (design decision): nothing
    // auto-transfers to treasury, the realised excess splits into YieldIndex
    // (stakers) and ProtocolYieldBalance (protocol, withdrawable later).
    // Only withdraw_yield ever moves tokens to treasury.
    // r3 (2026-09-29): `harvest` replaces the admin `extract_yield`. It
    // redeems only the growth; principal stays deployed at book value.
    let env = new_env();
    let s = setup(&env);
    // v1: these assertions cover the PROTOCOL's yield path, which only runs
    // when the protocol has a share. The v1 default is 100% to stakers, so
    // set 50% through the real settings timelock first.
    set_setting_via_timelock(&env, &s, crate::settings::SettingKey::StakerYieldBps, 5_000);
    staked_wallet(&env, &s);
    let (vault_id, mock) = with_vault(&env, &s, 8_000);
    let treasury = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Treasury(treasury.clone()));

    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);

    // 10% gain. The vault needs real XLM to pay above par, exactly as the
    // real one would after Blend accrues interest.
    mock.set_rate_bps(&11_000);
    s.token_admin.mint(&vault_id, &deployed);

    let shares = s.client.get_total_deployed_shares();
    let token = TokenClient::new(&env, &s.token_id);
    let yield_amount = s.client.harvest();

    // Shares redeemed are floored (never more than the growth is worth), so
    // the credit can come in a couple of stroops under the full 10%.
    assert!(yield_amount > 0 && deployed / 10 - yield_amount <= 2, "yield {}", yield_amount);
    assert_eq!(s.client.get_total_extracted_yield(), yield_amount);
    // Principal stays deployed at book value; only growth shares left.
    assert_eq!(s.client.get_total_deployed_asset(), deployed);
    assert!(s.client.get_total_deployed_shares() < shares);

    // Nothing reaches treasury at harvest time, gain or no gain.
    assert_eq!(token.balance(&treasury), 0);
    // Staker half is set aside (index-exact, dust to the protocol); the
    // protocol keeps the rest in the pool.
    let (staker_reserved, backer_reserved) = s.client.get_yield_reserved();
    assert_eq!(backer_reserved, 0);
    assert!(bps_of(yield_amount, 5_000) - staker_reserved <= 1);
    let protocol_share = yield_amount - staker_reserved;
    assert_eq!(s.client.get_yield_balance(), protocol_share);
    // Staker's half compounded into the index rather than transferring.
    assert!(s.client.get_yield_index() > YIELD_INDEX_PRECISION);

    // The protocol's credited half is real and withdrawable on demand.
    s.client.withdraw_yield(&protocol_share);
    assert_eq!(token.balance(&treasury), protocol_share);
    assert_eq!(s.client.get_yield_balance(), 0);
    assert_invariant(&s);
}

#[test]
fn a_venue_loss_yields_zero_rather_than_panicking() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (_vault_id, mock) = with_vault(&env, &s, 8_000);
    let treasury = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Treasury(treasury.clone()));

    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);

    // Blend bad debt: the position is worth 10% less than principal. Under
    // `overflow-checks = true` an unguarded `received - principal` would
    // panic here rather than produce a negative, V8's saturating form
    // (`:912`) is load-bearing, not cosmetic.
    mock.set_rate_bps(&9_000);

    let shares = s.client.get_total_deployed_shares();
    let yield_amount = s.client.harvest();

    assert_eq!(yield_amount, 0);
    // r3: a harvest on a loss redeems nothing (no growth to take).
    assert_eq!(s.client.get_total_deployed_shares(), shares);
    let token = TokenClient::new(&env, &s.token_id);
    assert_eq!(token.balance(&treasury), 0);
    assert_eq!(s.client.get_total_extracted_yield(), 0);
}

#[test]
fn min_xlm_out_floor_is_enforced_on_redemption() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (_v, mock) = with_vault(&env, &s, 8_000);

    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);
    mock.set_rate_bps(&9_000);
    // The real vault enforces the floor itself; switch that off to prove the
    // pool's own guard still catches it (defence in depth).
    mock.disable_min_check();

    let shares = s.client.get_total_deployed_shares();
    assert_eq!(
        s.client.try_provide_liquidity(&shares, &deployed),
        Err(Ok(PoolError::MinAmountNotMet))
    );
}

// -----------------------------------------------------------------------
// Treasury / yield views
// -----------------------------------------------------------------------

#[test]
fn withdraw_yield_requires_a_treasury_harvest_does_not() {
    // Renamed + rewritten 2026-09-18: crediting yield never touches
    // treasury (it only credits internal storage), so it must succeed
    // with none set. withdraw_yield is the only function that transfers, so
    // it is the only one that still needs a treasury configured.
    // r3: the crediting call is now the permissionless `harvest`.
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let (vault_id, mock) = with_vault(&env, &s, 5_000);
    s.client.deploy_to_vault(&(MID_STAKE / 4), &0);
    mock.set_rate_bps(&11_000);
    s.token_admin.mint(&vault_id, &MID_STAKE);

    assert!(s.client.try_harvest().is_ok());
    assert!(s.client.get_total_extracted_yield() > 0, "test is vacuous without a real harvest");
    assert_eq!(
        s.client.try_withdraw_yield(&1),
        Err(Ok(PoolError::TreasuryNotSet))
    );
}

#[test]
fn withdraw_yield_enforces_v8s_double_gate() {
    // Rewritten 2026-09-18: a raw donation no longer shows as yield, the
    // whole point of the explicit ProtocolYieldBalance counter is that only
    // an actual extract_yield call can credit it (see module docs on why the
    // residual formula was removed). Legitimate yield is generated for real
    // here instead of donated, and the donation case is kept as an explicit
    // assertion that it is now inert.
    let env = new_env();
    let s = setup(&env);
    // v1: these assertions cover the PROTOCOL's yield path, which only runs
    // when the protocol has a share. The v1 default is 100% to stakers, so
    // set 50% through the real settings timelock first.
    set_setting_via_timelock(&env, &s, crate::settings::SettingKey::StakerYieldBps, 5_000);
    staked_wallet(&env, &s);
    let (vault_id, mock) = with_vault(&env, &s, 8_000);
    let treasury = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Treasury(treasury.clone()));

    // No yield exists yet: everything liquid is staker principal.
    assert_eq!(s.client.get_yield_balance(), 0);
    assert_eq!(
        s.client.try_withdraw_yield(&1),
        Err(Ok(PoolError::ExceedsYieldBalance))
    );

    // A raw donation to the contract must NOT be withdrawable as yield,
    // the exploit the old residual formula allowed is now closed by
    // construction: nothing but extract_yield's own split can credit
    // ProtocolYieldBalance.
    let donation = STROOPS_PER_UNIT / 2;
    s.token_admin.mint(&s.client.address, &donation);
    assert_eq!(s.client.get_yield_balance(), 0);
    assert_eq!(
        s.client.try_withdraw_yield(&donation),
        Err(Ok(PoolError::ExceedsYieldBalance))
    );

    // Generate REAL yield via a harvest instead.
    let deployed = MID_STAKE * 8 / 10;
    s.client.deploy_to_vault(&deployed, &0);
    mock.set_rate_bps(&11_000);
    s.token_admin.mint(&vault_id, &deployed);
    let yield_amount = s.client.harvest();
    let (staker_reserved, _) = s.client.get_yield_reserved();
    let protocol_share = yield_amount - staker_reserved;
    assert!(protocol_share >= yield_amount - (yield_amount * 5_000 / 10_000));
    assert_eq!(s.client.get_yield_balance(), protocol_share);

    s.client.withdraw_yield(&protocol_share);
    let token = TokenClient::new(&env, &s.token_id);
    assert_eq!(token.balance(&treasury), protocol_share);
    assert_invariant(&s);
}

#[test]
fn yield_balance_reads_zero_while_principal_is_deployed() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);
    // liquid + deployed == staked exactly, so there is no excess.
    assert_eq!(s.client.get_yield_balance(), 0);
    assert_invariant(&s);
}

#[test]
fn deployment_ratio_can_drift_above_the_ceiling_after_a_withdrawal() {
    let env = new_env();
    let s = setup(&env);
    let (a, ben_a) = staked_wallet(&env, &s);
    let (_b, _ben_b) = staked_wallet(&env, &s);
    with_vault(&env, &s, 5_000);

    let total = s.client.get_total_staked();
    s.client.deploy_to_vault(&(total / 2), &0);
    assert_eq!(s.client.get_deployment_ratio_bps(), 5_000);

    // A exits. total_staked falls, deployed_asset does not, the ratio drifts
    // above deploy_bps. That is drift, not a breach: the invariant still
    // holds, and it is resolved by an admin rebalance rather than by
    // auto-unwinding from a user path.
    s.client.withdraw(&a, &ben_a);
    assert!(s.client.get_deployment_ratio_bps() > 5_000);
    assert_invariant(&s);
}

// -----------------------------------------------------------------------
// Regression: the accounting layer must not disturb pool mechanics
// -----------------------------------------------------------------------

#[test]
fn solvency_and_cap_math_still_read_total_staked_not_liquid() {
    let env = new_env();
    let s = setup(&env);
    let (wallet, _b) = staked_wallet(&env, &s);
    with_vault(&env, &s, 8_000);

    let staked_before = s.client.get_total_staked();
    s.client.deploy_to_vault(&(MID_STAKE * 8 / 10), &0);

    // Deployment moves XLM out of the contract but must not move any
    // accounting figure the claim path reasons over.
    assert_eq!(s.client.get_total_staked(), staked_before);
    assert_eq!(s.client.get_total_allocated(), 0);

    // A claim is still admissible even though most of the XLM is in the
    // vault: solvency is an economic question, not a liquidity one.
    //
    // MID_STAKE / 4 is the largest single-day entitlement `claim::stress_cap`
    // admits at zero utilization (2,500 bps of total_staked). It is also
    // MORE than what is still liquid after an 8,000-bps deployment, which is
    // the sharper version of this test's point: the claim is admitted on
    // accounting figures that are blind to where the XLM physically sits.
    // Liquidity is enforced later, at payout, and has its own test above.
    advance_days(&env, 91);
    let entitlement = MID_STAKE / 4;
    let claim_id = submit_claim_signed(
        &env,
        &s,
        &s.oracle.clone(),
        &wallet,
        &tx_hash(&env, 3),
        &entitlement,
        &1u32,
        &now_ts(&env),
    );
    assert_eq!(s.client.get_total_allocated(), entitlement);
    assert!(entitlement > s.client.get_liquid_balance());
    assert!(s.client.get_claim(&claim_id).is_some());
    assert_invariant(&s);
}
