//! D2 (Tranche 2), DeFindex vault yield deployment.
//!
//! Ports V8's liquid-vs-deployed ACCOUNTING (`SAFUPoolV8.sol:851-921`) and
//! deliberately REJECTS V8's deployment POLICY. See types.rs's D2 block for
//! the full reasoning; the short version is that V8 can deploy 100% inline
//! because it tracks a per-staker wstETH tranche and unwinds it inside
//! `withdraw()`, whereas this contract has one pooled position and two
//! withdrawal paths with no cooldown to react inside, `stake::withdraw`
//! (no time lock) and `stake::emergency_exit` (runs while paused).
//!
//! Design locked by the 2026-08-14 engineering review:
//!
//! 1. The FIRST deposit is a separate admin call (`deploy_to_vault`): it
//!    sets the reference rate every later deposit is checked against.
//! 2. Bounded by `deploy_bps` (hard-capped at `MAX_DEPLOY_BPS`) and floored
//!    so it can never deploy XLM already reserved as `total_allocated`.
//! 3. SUPERSEDED 2026-09-24 (design and engineering review): was "never auto-unwound from a user path".
//!    The pool now rebalances itself inside user calls, both ways:
//!    - money out (`withdraw`, `claim_stream`, `emergency_exit`,
//!      `complete_backer_withdrawal`): `pull_for_payment` redeems from the
//!      vault when cash is short, refilling the buffer, then pays;
//!    - money in (`stake`, `back`): `push_idle` deposits idle cash above
//!      the buffer.
//!    Both use `try_` vault calls: a vault failure never changes pool state.
//!    A failed pull ends as `InsufficientLiquidity`, as before; a failed
//!    push is skipped and the stake or deposit still succeeds (a CCTP mint
//!    must never revert). The permissionless `ensure_liquidity` /
//!    `auto_deploy_liquidity` stay, so a keeper is optional.
//! 4. `deployed_asset` is held at original deposit value, never marked to
//!    market, so no externally mutable value feeds a payout decision.
//!
//! **The invariant this module must preserve:**
//!
//! ```text
//! liquid_balance + deployed_asset  >=  total_staked
//! ```
//!
//! V8's form carries `+ totalFailedPayouts` on the right (`:874`); T1
//! deliberately skipped that machinery (§5b of the Soroban KB), so the
//! Soroban form simplifies as above. Note §5b's ORIGINAL justification,
//! "the contract always holds what it owes, so a native XLM transfer
//! essentially cannot fail": is void from D2 onward, exactly as its
//! `revokedApprovals` sibling was voided by D1. The decision to skip the
//! rescue bucket still stands, but for a different reason: Soroban has no
//! EVM-style partial-failure mode where a transfer fails while the rest of
//! the transaction succeeds. A short balance is caught by an explicit
//! pre-check returning `InsufficientLiquidity` BEFORE any state is written,
//! and the caller simply retries once admin has rebalanced.
//!
//! **Vault interface** verified 2026-08-14 against the deployed contract's
//! own embedded spec over Soroban RPC (`stellar contract info interface
//! --id CCLV4H7WTLJQ7ATLHBBQV2WW3OINF3FOY5XZ7VPHZO7NH3D2ZS4GFSF6 --network
//! testnet`), not from documentation and not from GitHub.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contractclient, contractevent, token::TokenClient, vec, Address, Env, IntoVal, Symbol, Val, Vec,
};

use crate::error::PoolError;
use crate::storage;
use crate::types::{
    AUTO_PUSH_MIN_BPS, BPS_DENOMINATOR, DEPLOY_BPS_DENOMINATOR, HARVEST_MAX_GROWTH_BPS_PER_DAY,
    HARVEST_SLIPPAGE_BPS, MAX_REBALANCE_SLIPPAGE_BPS, SECONDS_PER_DAY, YIELD_INDEX_PRECISION,
    YIELD_SPLIT_BPS_DENOMINATOR,
};

// -----------------------------------------------------------------------
// Vault client: minimal by design.
//
// Only the three functions D2 actually needs are declared. The real vault
// exposes ~40 (full SAC token surface, Soroswap library helpers, role
// management, rebalancing); declaring only what we call keeps our exposure
// to a DeFindex interface change as small as possible.
//
// `deposit`'s real return is
//   Result<(Vec<i128>, i128, Option<Vec<Option<AssetInvestmentAllocation>>>), ContractError>
// whose third element is a DeFindex-internal type. Declared as `Val` and
// ignored: shares gained are measured by balance delta instead, which is
// V8's own pattern (`:296-298`, "delta pattern, F10: concurrent-stake
// safe") and avoids importing a foreign type into our contract purely to
// discard it.
// -----------------------------------------------------------------------

// `#[contractclient]` consumes this trait to GENERATE `VaultClient`, which
// is what the rest of the module calls. The trait itself is never invoked
// directly, so rustc's dead-code pass flags it, expected for this macro,
// and suppressed narrowly here rather than by relaxing lints crate-wide.
// D1 landed at zero warnings; this keeps that.
#[allow(dead_code)]
#[contractclient(name = "VaultClient")]
pub trait VaultInterface {
    fn deposit(
        env: Env,
        amounts_desired: Vec<i128>,
        amounts_min: Vec<i128>,
        from: Address,
        invest: bool,
    ) -> Val;

    fn withdraw(
        env: Env,
        withdraw_shares: i128,
        min_amounts_out: Vec<i128>,
        from: Address,
    ) -> Val;

    /// The vault is itself a token; dfToken balance IS the share position.
    fn balance(env: Env, id: Address) -> i128;

    /// r3 (2026-09-29): current asset value of `vault_shares`, one entry per
    /// vault asset (single-asset here). Verified against the deployed
    /// testnet vault's own spec over RPC (`stellar contract info interface`):
    /// `get_asset_amounts_per_shares(vault_shares: i128) -> Result<Vec<i128>, ContractError>`.
    /// Read only by `harvest`, never by any payout or solvency decision.
    fn get_asset_amounts_per_shares(env: Env, vault_shares: i128) -> Vec<i128>;
}

// -----------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------

#[contractevent]
pub struct Deployed {
    #[topic]
    pub vault: Address,
    pub xlm_amount: i128,
    pub shares_gained: i128,
}

#[contractevent]
pub struct LiquidityProvided {
    #[topic]
    pub vault: Address,
    pub shares_redeemed: i128,
    pub asset_received: i128,
}

/// T3 (2026-08-24), `ensure_liquidity`'s permissionless-triggered pull.
/// Distinct from `LiquidityProvided` so on-chain monitoring can tell an
/// automatic rebalance apart from an admin's manual `provide_liquidity`.
#[contractevent]
pub struct LiquidityAutoRebalanced {
    #[topic]
    pub vault: Address,
    pub shares_redeemed: i128,
    pub asset_received: i128,
}

/// T3 (2026-08-24), `auto_deploy_liquidity`'s permissionless-triggered
/// push, the deposit-side mirror of `LiquidityAutoRebalanced`.
#[contractevent]
pub struct LiquidityAutoDeployed {
    #[topic]
    pub vault: Address,
    pub xlm_amount: i128,
    pub shares_gained: i128,
}

/// r3 (2026-09-29): emitted every time vault growth is recognised as yield,
/// whichever path redeemed it (harvest, in-path pull, keeper, admin).
/// Replaces `YieldExtracted`. Nothing transfers here.
#[contractevent]
pub struct YieldCredited {
    pub yield_amount: i128,
    pub staker_share: i128,
    pub backer_share: i128,
    pub protocol_share: i128,
}

/// r3: a staker's or backer's yield paid out.
#[contractevent]
pub struct YieldPaid {
    #[topic]
    pub owner: Address,
    pub to: Address,
    pub amount: i128,
}

#[contractevent]
pub struct YieldWithdrawn {
    #[topic]
    pub treasury: Address,
    pub amount: i128,
}

/// Emitted when a redemption returns LESS XLM than the proportional
/// principal that was deployed: i.e. the venue lost money (Blend bad debt
/// or an adverse share-price move).
///
/// V8 has no equivalent. Its `extractYield` handles the same case only by
/// declining to extract (`:912`, `receivedEth > ethEquiv ? ... : 0`), so a
/// principal shortfall is silently absorbed and only surfaces much later as
/// an inability to pay a claim. Emitting a distinct event costs nothing and
/// turns a silent loss into something monitorable off-chain. The full
/// remedy is T3 scope; this is the cheap detection half.
#[contractevent]
pub struct DeploymentShortfall {
    #[topic]
    pub vault: Address,
    pub principal_expected: i128,
    pub asset_received: i128,
}

/// v1 (2026-09-24): an in-path push got fewer shares than the contract's
/// own reference rate allows. The shares actually received are recorded
/// (accounting stays true, design decision A); this makes it visible.
#[contractevent]
pub struct PushBelowFloor {
    #[topic]
    pub vault: Address,
    pub xlm_amount: i128,
    pub shares_gained: i128,
    pub min_shares: i128,
}

// -----------------------------------------------------------------------
// Liquidity helpers: used by stake.rs and claim.rs before every outbound
// transfer.
// -----------------------------------------------------------------------

/// The contract's REAL liquid XLM balance, as opposed to `total_staked`
/// (an accounting figure that includes XLM sitting in the vault).
///
/// This is the contract's first-ever balance read; before D2 the two were
/// identical by construction. Note the same caveat V8 documents at `:853`:
/// XLM sent to the contract outside `stake()` inflates this figure and
/// therefore appears as extractable yield. Harmless, it cannot affect
/// `total_staked` and so cannot affect solvency, but worth knowing.
pub fn liquid_balance(env: &Env) -> i128 {
    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.balance(&env.current_contract_address())
}

/// Fail with a typed `InsufficientLiquidity` rather than letting the SAC
/// transfer trap opaquely. Called immediately before every outbound
/// transfer in `stake.rs` and `claim.rs`.
pub fn require_liquidity(env: &Env, amount: i128) -> Result<(), PoolError> {
    if free_liquid(env) < amount {
        return Err(PoolError::InsufficientLiquidity);
    }
    Ok(())
}

/// r3 (2026-09-29): liquid cash minus yield owed to stakers and backers.
/// Every payment other than a yield payout, and every push into the vault,
/// is bounded by this, so set-aside yield is never spent on anything else.
/// The protocol's own yield share is NOT set aside (design decision
/// 2026-09-29): it is pool cash, usable for claims until withdrawn.
pub fn free_liquid(env: &Env) -> i128 {
    liquid_balance(env) - storage::get_yield_reserved(env)
}

// -----------------------------------------------------------------------
// In-path rebalancing (v1, 2026-09-24): see rule 3 in the module doc.
// -----------------------------------------------------------------------

/// Called before every outbound transfer in place of `require_liquidity`.
/// If cash is short, redeem the shortfall plus a refill up to the buffer
/// (`capacity × (1 − deploy_bps)`); if the vault can't give that much, try
/// the shortfall alone. Then the usual check: pay, or `InsufficientLiquidity`.
pub fn pull_for_payment(env: &Env, amount: i128) -> Result<(), PoolError> {
    let liquid = free_liquid(env);
    if liquid < amount {
        let shortfall = amount - liquid;
        let buffer = storage::get_capacity(env)
            * (DEPLOY_BPS_DENOMINATOR - storage::get_deploy_bps(env))
            / DEPLOY_BPS_DENOMINATOR;
        if !try_pull(env, shortfall + buffer.max(0)) {
            try_pull(env, shortfall);
        }
    }
    require_liquidity(env, amount)
}

/// Redeem enough shares for `target` XLM (rounded up, capped at the whole
/// position). The vault enforces the 5% floor itself (`min_amounts_out`), and
/// a failed `try_withdraw` leaves nothing behind. Returns true if it redeemed.
fn try_pull(env: &Env, target: i128) -> bool {
    let Some(vault_addr) = storage::get_vault(env) else {
        return false;
    };
    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);
    if target <= 0 || deployed_shares <= 0 || deployed_asset <= 0 {
        return false;
    }
    // Round up so book-value rounding can't leave the payment short.
    let shares = ((target * deployed_shares + deployed_asset - 1) / deployed_asset).min(deployed_shares);
    let expected = deployed_asset * shares / deployed_shares;
    let mut mins = Vec::new(env);
    mins.push_back(expected * (BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS) / BPS_DENOMINATOR);

    let liquid_before = liquid_balance(env);
    authorize_withdraw(env, &vault_addr, shares, &mins);
    let vault = VaultClient::new(env, &vault_addr);
    if vault.try_withdraw(&shares, &mins, &env.current_contract_address()).is_err() {
        return false;
    }
    let asset_received = liquid_balance(env) - liquid_before;
    record_redeem(env, &vault_addr, shares, asset_received);
    LiquidityAutoRebalanced { vault: vault_addr, shares_redeemed: shares, asset_received }.publish(env);
    true
}

/// Called after the inbound transfer in `stake` and `back`. Deposits idle
/// cash (above what live claims need) up to the `deploy_bps` line, once that
/// is at least `AUTO_PUSH_MIN_BPS` of capacity. Never fails: if anything is
/// off (paused, no vault, no reference deposit yet, vault error) it returns
/// and the cash stays in the pool.
pub fn push_idle(env: &Env) {
    if storage::is_paused(env) {
        return;
    }
    let Some(vault_addr) = storage::get_vault(env) else {
        return;
    };
    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);
    if deployed_shares <= 0 || deployed_asset <= 0 {
        return; // the first deposit is the admin's (reference rate)
    }
    let capacity = storage::get_capacity(env);
    let idle = (free_liquid(env) - storage::get_total_allocated(env)).max(0);
    let room = (capacity * storage::get_deploy_bps(env) / DEPLOY_BPS_DENOMINATOR - deployed_asset).max(0);
    let amount = idle.min(room);
    if amount <= 0 || amount < capacity * AUTO_PUSH_MIN_BPS / BPS_DENOMINATOR {
        return;
    }
    let min_shares = amount * deployed_shares / deployed_asset * (BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS)
        / BPS_DENOMINATOR;

    let vault = VaultClient::new(env, &vault_addr);
    let Ok(Ok(shares_before)) = vault.try_balance(&env.current_contract_address()) else {
        return;
    };
    let mut desired = Vec::new(env);
    desired.push_back(amount);
    let mut mins = Vec::new(env);
    mins.push_back(amount);
    authorize_deposit(env, &vault_addr, &desired, &mins, amount);
    if vault.try_deposit(&desired, &mins, &env.current_contract_address(), &true).is_err() {
        return;
    }
    // The deposit just succeeded, so a plain read is safe here.
    let shares_gained = vault.balance(&env.current_contract_address()) - shares_before;
    record_deposit(env, amount, shares_gained);
    if shares_gained < min_shares {
        PushBelowFloor { vault: vault_addr.clone(), xlm_amount: amount, shares_gained, min_shares }.publish(env);
    }
    LiquidityAutoDeployed { vault: vault_addr, xlm_amount: amount, shares_gained }.publish(env);
}

/// Shared accounting for every deposit into the vault.
fn record_deposit(env: &Env, amount: i128, shares_gained: i128) {
    // CSO M2: the harvest growth limit's clock starts with the first money deployed.
    if storage::get_last_harvest_at(env) == 0 {
        storage::set_last_harvest_at(env, env.ledger().timestamp().max(1));
    }
    storage::set_total_deployed_shares(env, storage::get_total_deployed_shares(env) + shares_gained);
    storage::set_total_deployed_asset(env, storage::get_total_deployed_asset(env) + amount);
    storage::bump_instance_ttl(env);
}

/// Shared accounting for every redemption: book value out, and a loss (if
/// any) marked down in `total_staked` (T3 fix, see `redeem`). Returns the
/// principal equivalent of the redeemed shares.
fn record_redeem(env: &Env, vault_addr: &Address, shares: i128, asset_received: i128) -> i128 {
    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);
    let principal_equiv = deployed_asset * shares / deployed_shares;
    storage::set_total_deployed_shares(env, deployed_shares - shares);
    storage::set_total_deployed_asset(env, deployed_asset - principal_equiv);
    // r3 (2026-09-29): growth that comes back with any redemption is yield.
    // Before r3 it landed as unowned cash and `push_idle` re-deposited it as
    // principal, so it reached nobody.
    if asset_received > principal_equiv {
        credit_yield(env, asset_received - principal_equiv);
    }
    if asset_received < principal_equiv {
        let shortfall = principal_equiv - asset_received;
        storage::set_total_staked(env, storage::get_total_staked(env).saturating_sub(shortfall));
        DeploymentShortfall {
            vault: vault_addr.clone(),
            principal_expected: principal_equiv,
            asset_received,
        }
        .publish(env);
    }
    storage::bump_instance_ttl(env);
    principal_equiv
}

// -----------------------------------------------------------------------
// Views
// -----------------------------------------------------------------------

// REMOVED 2026-09-18: `yield_balance()`'s residual formula ("everything
// the pool controls minus what it owes stakers") had a documented real bug
// (2026-08-20: two activated claims misread as +4,100 XLM "yield" that was
// really forfeited principal). Replaced by `storage::get_protocol_yield_balance`,
// an explicit increment/decrement counter that cannot drift the same way,
// it is credited only by `credit_yield`'s protocol-share split and debited
// only by `withdraw_yield`, with no dependency on `total_allocated` timing
// at all. `get_yield_balance()` in lib.rs now reads that counter directly.

/// A staker's current withdrawable value: principal plus their proportional
/// share of yield realised since they staked. Exposed as a read-only helper
/// so a caller (frontend or otherwise) never has to re-derive the
/// index-ratio formula itself: the same category of bug as the USTRY/8-vs-7
/// decimals mistake happens when a formula is copied instead of called.
pub fn withdrawable_amount(env: &Env, staker: &Address) -> i128 {
    match storage::get_stake(env, staker) {
        Some(record) if record.amount > 0 && !record.withdrawn => {
            record.amount + staker_yield_owed(env, &record)
        }
        _ => 0,
    }
}

/// How much of `total_staked` is currently in the vault, in bps.
///
/// Can read ABOVE `deploy_bps` without anything being wrong: the ceiling is
/// checked prospectively at `deploy_to_vault`, so a large withdrawal
/// afterwards lowers `total_staked` while `deployed_asset` is unchanged. That
/// is drift, not a breach, the solvency invariant still holds, and is
/// resolved by `ensure_liquidity` or the next in-path pull (v1, 2026-09-24).
pub fn deployment_ratio_bps(env: &Env) -> i128 {
    let capacity = storage::get_capacity(env);
    if capacity <= 0 {
        return 0;
    }
    storage::get_total_deployed_asset(env) * DEPLOY_BPS_DENOMINATOR / capacity
}

// -----------------------------------------------------------------------
// Admin configuration
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// Cross-contract authorization
//
// When this contract calls `vault.deposit(from = self, ..)`, the vault
// re-enters the token to pull the XLM. Those `require_auth` calls resolve to
// THIS contract, and Soroban does not propagate a contract's own authority
// into sub-invocations automatically: it has to be declared up front via
// `authorize_as_current_contract` (Soroban vuln checklist #2).
//
// The tree below is VERIFIED, not assumed. On 2026-08-14 a `deposit` was
// simulated against the LIVE testnet vault over Soroban RPC
// (`simulateTransaction`) and the auth entries it returned were decoded:
//
//     deposit(vault, [amounts_desired, amounts_min, from, invest])
//       └── transfer(token, [from, vault, amount])
//
// That settles both questions D2 had recorded as open:
//
//   1. DeFindex's `deposit` calls `from.require_auth()` ITSELF, so the tree
//      is genuinely two levels deep. A single top-level `transfer` entry
//      does NOT satisfy it: the transfer entry must hang beneath the
//      `deposit` context or it is never reached.
//   2. The pull is `transfer(from, vault, amount)`, NOT `transfer_from`.
//
// `withdraw` was verified the same way, and needed a real position to do it.
// The vault reverts at its own share-balance check before reaching any
// sub-invocation, so a simulating account holding zero shares learns
// nothing; a 1 XLM deposit was submitted from the `admin` testnet identity
// (tx `1d0232a7e50f2a8f0e2a1448da64ede3b5332b814676cc03e7d2dd418483a058`)
// to create one. The tree it then returned is ONE level:
//
//     withdraw(vault, [withdraw_shares, min_amounts_out, from])
//
// with NO sub-invocations. That confirms the mechanism-based reading: the
// XLM leg of a redemption moves the VAULT's own funds to us, and a contract
// self-authorizes movement of its own balance, so no nested entry of ours
// applies. The testnet position was left in place deliberately, it is what
// makes this simulation repeatable at D4.
//
// Note the real vault does NOT mint shares 1:1 (that deposit returned 5,996
// dfTokens for 10,000,000 stroops). Irrelevant to correctness here because
// `deploy_to_vault` measures shares by balance delta rather than assuming a
// rate, but worth knowing before reading the mock's 1:1 rate as realistic.
//
// Both helpers must be called IMMEDIATELY before their vault call:
// `authorize_as_current_contract` applies to the next contract invocation
// only, so an intervening call (e.g. the `balance` read) would consume it.
// -----------------------------------------------------------------------

fn authorize_deposit(
    env: &Env,
    vault_addr: &Address,
    desired: &Vec<i128>,
    mins: &Vec<i128>,
    amount: i128,
) {
    let pool = env.current_contract_address();

    let mut transfer_args: Vec<Val> = Vec::new(env);
    transfer_args.push_back(pool.clone().into_val(env));
    transfer_args.push_back(vault_addr.clone().into_val(env));
    transfer_args.push_back(amount.into_val(env));

    let mut deposit_args: Vec<Val> = Vec::new(env);
    deposit_args.push_back(desired.clone().into_val(env));
    deposit_args.push_back(mins.clone().into_val(env));
    deposit_args.push_back(pool.into_val(env));
    deposit_args.push_back(true.into_val(env));

    let token = storage::get_asset_token(env);

    // The nested form is what the LIVE vault requires (see the block comment
    // above). The flat form is what the test harness requires, because
    // `mock_all_auths()` satisfies the vault's own `deposit` require_auth
    // without consuming the entry above it, which leaves anything nested
    // beneath that entry unreachable. Declaring both makes this correct in
    // both environments; the one that does not apply is simply never
    // matched, which the host tolerates. Verified empirically in each
    // direction rather than assumed: the flat entry alone fails on the
    // simulated testnet tree, and the nested entry alone fails under
    // `mock_all_auths`.
    env.authorize_as_current_contract(vec![
        env,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: vault_addr.clone(),
                fn_name: Symbol::new(env, "deposit"),
                args: deposit_args,
            },
            sub_invocations: vec![
                env,
                InvokerContractAuthEntry::Contract(SubContractInvocation {
                    context: ContractContext {
                        contract: token.clone(),
                        fn_name: Symbol::new(env, "transfer"),
                        args: transfer_args.clone(),
                    },
                    sub_invocations: vec![env],
                }),
            ],
        }),
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: token,
                fn_name: Symbol::new(env, "transfer"),
                args: transfer_args,
            },
            sub_invocations: vec![env],
        }),
    ]);
}

fn authorize_withdraw(env: &Env, vault_addr: &Address, shares: i128, mins: &Vec<i128>) {
    let mut withdraw_args: Vec<Val> = Vec::new(env);
    withdraw_args.push_back(shares.into_val(env));
    withdraw_args.push_back(mins.clone().into_val(env));
    withdraw_args.push_back(env.current_contract_address().into_val(env));

    env.authorize_as_current_contract(vec![
        env,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: vault_addr.clone(),
                fn_name: Symbol::new(env, "withdraw"),
                args: withdraw_args,
            },
            sub_invocations: vec![env],
        }),
    ]);
}

// -----------------------------------------------------------------------
// deploy_to_vault: admin-triggered, bounded, the ONLY inbound path to the
// vault. Deliberately absent from stake().
// -----------------------------------------------------------------------

pub fn deploy_to_vault(
    env: &Env,
    amount: i128,
    min_shares_out: i128,
) -> Result<i128, PoolError> {
    storage::require_not_paused(env)?;
    let admin = storage::get_admin(env);
    admin.require_auth();

    if amount <= 0 {
        return Err(PoolError::AmountNotPositive);
    }
    let vault_addr = storage::get_vault(env).ok_or(PoolError::VaultNotSet)?;

    // r3: set-aside yield is never deployed.
    let liquid = free_liquid(env);
    if liquid < amount {
        return Err(PoolError::InsufficientLiquidity);
    }

    // Ceiling: never deploy more than deploy_bps of capacity (v1: staker
    // principal + matured backer money).
    let deployed_asset = storage::get_total_deployed_asset(env);
    let ceiling = storage::get_capacity(env) * storage::get_deploy_bps(env) / DEPLOY_BPS_DENOMINATOR;
    if deployed_asset + amount > ceiling {
        return Err(PoolError::DeployExceedsCeiling);
    }

    // Floor: never deploy XLM already reserved for a live claim. This is
    // what guarantees claim_stream's liquidity pre-check can always be
    // satisfied for entitlements already admitted.
    if liquid - amount < storage::get_total_allocated(env) {
        return Err(PoolError::DeployBreachesAllocation);
    }

    let vault = VaultClient::new(env, &vault_addr);
    let shares_before = vault.balance(&env.current_contract_address());

    // Single-asset vault (XLM only), so both vectors are one element.
    // amounts_min == amounts_desired: this bound is on what leaves US, and
    // a partial deposit would desynchronise the delta accounting below.
    let mut desired = Vec::new(env);
    desired.push_back(amount);
    let mut mins = Vec::new(env);
    mins.push_back(amount);

    authorize_deposit(env, &vault_addr, &desired, &mins, amount);

    vault.deposit(
        &desired,
        &mins,
        &env.current_contract_address(),
        &true, // invest immediately rather than sitting as vault idle_funds
    );

    let shares_gained = vault.balance(&env.current_contract_address()) - shares_before;
    if shares_gained < min_shares_out {
        return Err(PoolError::MinSharesNotMet);
    }

    record_deposit(env, amount, shares_gained);

    Deployed {
        vault: vault_addr,
        xlm_amount: amount,
        shares_gained,
    }
    .publish(env);

    Ok(shares_gained)
}

/// T3 (2026-08-24), the permissionless sibling of `deploy_to_vault`, the
/// deposit-side mirror of `ensure_liquidity`. Locked scope 2026-08-24: put
/// idle liquidity to work automatically once staking inflow builds up,
/// same "nothing caller-controlled" rule as the pull direction.
///
/// **Bootstrapping constraint:** the vault exposes no on-chain price quote,
/// so there's nothing to check a deposit's exchange rate against until the
/// contract has its OWN prior deposit to reference
/// (`deployed_asset / deployed_shares`). The very first deposit into a fresh
/// vault still has to be the existing manual `deploy_to_vault` call, this
/// function only activates once `deployed_shares > 0`.
///
/// `require_not_paused` stays, mirroring `deploy_to_vault`: unlike
/// pulling liquidity back (the pause-time escape path), pushing MORE money
/// into a third-party venue while something's wrong should not happen
/// automatically.
pub fn auto_deploy_liquidity(env: &Env) -> Result<i128, PoolError> {
    storage::require_not_paused(env)?;

    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);
    if deployed_shares <= 0 || deployed_asset <= 0 {
        // Bootstrapping: no prior deposit to reference a safe rate against.
        return Err(PoolError::NothingDeployed);
    }

    // r3: set-aside yield is never deployed.
    let liquid = free_liquid(env);
    let total_allocated = storage::get_total_allocated(env);
    let idle = (liquid - total_allocated).max(0);
    if idle == 0 {
        return Ok(0);
    }

    let ceiling = storage::get_capacity(env) * storage::get_deploy_bps(env) / DEPLOY_BPS_DENOMINATOR;
    let room = (ceiling - deployed_asset).max(0);
    if room == 0 {
        return Ok(0);
    }

    let amount = idle.min(room);

    let vault_addr = storage::get_vault(env).ok_or(PoolError::VaultNotSet)?;

    // Floor: never deploy XLM reserved for a live claim, same guard
    // `deploy_to_vault` enforces. Structurally unreachable given `idle`'s
    // computation above (kept as defence in depth, same style as the rest
    // of this module, against the two figures drifting apart between the
    // read and the deposit).
    if liquid - amount < total_allocated {
        return Err(PoolError::DeployBreachesAllocation);
    }

    // Reference rate from the contract's own last-known deposits, the
    // only price data available on-chain (see doc comment above).
    let expected_shares = amount * deployed_shares / deployed_asset;
    let min_shares_out =
        expected_shares * (BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS) / BPS_DENOMINATOR;

    let vault = VaultClient::new(env, &vault_addr);
    let shares_before = vault.balance(&env.current_contract_address());

    let mut desired = Vec::new(env);
    desired.push_back(amount);
    let mut mins = Vec::new(env);
    mins.push_back(amount);

    authorize_deposit(env, &vault_addr, &desired, &mins, amount);

    vault.deposit(
        &desired,
        &mins,
        &env.current_contract_address(),
        &true,
    );

    let shares_gained = vault.balance(&env.current_contract_address()) - shares_before;
    if shares_gained < min_shares_out {
        return Err(PoolError::MinSharesNotMet);
    }

    record_deposit(env, amount, shares_gained);

    LiquidityAutoDeployed {
        vault: vault_addr,
        xlm_amount: amount,
        shares_gained,
    }
    .publish(env);

    Ok(shares_gained)
}

// -----------------------------------------------------------------------
// Redemption: shared core for provide_liquidity and ensure_liquidity.
// -----------------------------------------------------------------------

/// Redeems `shares`, returns `(asset_received, principal_equivalent)`.
///
/// `principal_equivalent` is the proportional slice of `deployed_asset` this
/// tranche represents, at ORIGINAL deposit value, V8's `ethEquiv`
/// (`:881`/`:903`). Multiplication before division throughout (Soroban vuln
/// checklist #3: `(a / b) * c` truncates to zero where `(a * c) / b` does
/// not).
fn redeem(
    env: &Env,
    vault_addr: &Address,
    shares: i128,
    min_asset_out: i128,
) -> Result<(i128, i128), PoolError> {
    if shares <= 0 {
        return Err(PoolError::AmountNotPositive);
    }
    let deployed_shares = storage::get_total_deployed_shares(env);
    if deployed_shares <= 0 {
        return Err(PoolError::NothingDeployed);
    }
    if shares > deployed_shares {
        return Err(PoolError::RedeemExceedsDeployed);
    }

    let deployed_asset = storage::get_total_deployed_asset(env);
    let principal_equiv = deployed_asset * shares / deployed_shares;

    let vault = VaultClient::new(env, vault_addr);
    let liquid_before = liquid_balance(env);

    let mut mins = Vec::new(env);
    mins.push_back(min_asset_out);

    authorize_withdraw(env, vault_addr, shares, &mins);

    vault.withdraw(&shares, &mins, &env.current_contract_address());

    let asset_received = liquid_balance(env) - liquid_before;
    if asset_received < min_asset_out {
        return Err(PoolError::MinAmountNotMet);
    }

    // T3 fix (2026-08-24), now in `record_redeem`: a real vault-level loss
    // (DeFindex/Blend bad debt or slippage beyond the floor) is marked down
    // in total_staked as well as in the DeploymentShortfall event. Before
    // that, the solvency check (`total_allocated <= total_staked`, claim.rs)
    // evaluated future claims against a figure that overstated what the pool
    // holds. Pool-wide aggregate only, never a per-staker balance, so it
    // tightens the ceiling for FUTURE claims and cannot shrink an
    // already-Active claim's snapshotted `entitlement`.
    record_redeem(env, vault_addr, shares, asset_received);

    Ok((asset_received, principal_equiv))
}

/// V8 `provideClaimLiquidity` (`:876`), redeem from the vault so the
/// redeemed XLM sits liquid in the contract, ready to fund payouts and
/// withdrawals. Nothing leaves the pool.
///
/// **Deliberate deviation from V8: no `require_not_paused`.** V8 marks its
/// equivalent `whenNotPaused`. Porting that modifier here would be a real
/// defect: `stake::emergency_exit` is specifically the pause-time escape
/// hatch, so if liquid XLM is short during a pause the operator would have
/// no way to fund the very function stakers are relying on. V8's own
/// `extractYield`/`withdrawYield` already omit the modifier for a related
/// reason (`:895`).
pub fn provide_liquidity(
    env: &Env,
    shares: i128,
    min_asset_out: i128,
) -> Result<i128, PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    let vault_addr = storage::get_vault(env).ok_or(PoolError::VaultNotSet)?;
    let (asset_received, _principal) = redeem(env, &vault_addr, shares, min_asset_out)?;
    storage::bump_instance_ttl(env);

    LiquidityProvided {
        vault: vault_addr,
        shares_redeemed: shares,
        asset_received,
    }
    .publish(env);

    Ok(asset_received)
}

/// T3 (2026-08-24), the permissionless sibling of `provide_liquidity`.
/// Locked design (2026-08-20 job): the CONTRACT computes both the amount
/// and the slippage floor: nothing caller-controlled, so a public
/// function can't be used to force an unwind at a bad price for zero
/// personal gain (the naive "just make `provide_liquidity` public" version
/// was flagged unsafe for exactly this reason).
///
/// Target is `total_allocated`: the same figure `deploy_to_vault` already
/// protects on the way IN (`DeployBreachesAllocation`: never deploy XLM
/// reserved for a live claim). This closes the gap the other direction:
/// pull back only enough to cover what's currently reserved, no more.
///
/// No `require_not_paused`, same reasoning as `provide_liquidity` above,
/// this IS the pause-time liquidity-restoring path.
pub fn ensure_liquidity(env: &Env) -> Result<i128, PoolError> {
    // r3: set-aside yield cannot cover claims, so it does not count here.
    let liquid = free_liquid(env);
    let total_allocated = storage::get_total_allocated(env);
    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);

    // TWO independent reasons to pull, added 2026-08-24, the function
    // originally covered only the first, which left the pair asymmetric:
    // `auto_deploy_liquidity` pushed against the `deploy_bps` line while
    // this pulled against an unrelated absolute figure.
    //
    // 1. CLAIMS SHORTFALL: liquid XLM is below what active claims are
    //    owed. Absolute, not proportional: what matters is that a payout
    //    can physically settle.
    let claims_shortfall = (total_allocated - liquid).max(0);
    //
    // 2. OVER-CEILING DRIFT: the vault position is a larger share of the
    //    pool than `deploy_bps` allows. This is the case identified
    //    2026-08-24: stakers withdrawing shrinks `total_staked`
    //    while `deployed_asset` is unchanged, so the RATIO climbs above the
    //    configured line without a single new deployment. `vault.rs`
    //    previously documented this as "drift, not a breach... resolved by
    //    an admin `provide_liquidity`": i.e. it needed a human. Now the
    //    same `deploy_bps` number governs both directions and it
    //    self-corrects.
    let ceiling = storage::get_capacity(env) * storage::get_deploy_bps(env)
        / DEPLOY_BPS_DENOMINATOR;
    let over_ceiling = (deployed_asset - ceiling).max(0);

    // Whichever need is larger: satisfying the bigger one satisfies both.
    let shortfall = claims_shortfall.max(over_ceiling);
    if shortfall == 0 {
        return Ok(0);
    }

    if deployed_shares <= 0 || deployed_asset <= 0 {
        // Nothing deployed to pull from, `redeem` would report this
        // itself, but returning it directly here avoids computing a
        // division against a zero denominator below.
        return Err(PoolError::NothingDeployed);
    }

    // Never try to redeem more than what's actually deployed, a shortfall
    // larger than the vault position is a real "money nowhere" case
    // `redeem`'s own `RedeemExceedsDeployed` guard would catch anyway;
    // capping here just picks the best-effort amount instead of erroring
    // out entirely when a partial rescue is still possible.
    let shares_needed = (shortfall * deployed_shares / deployed_asset).min(deployed_shares);
    let expected_asset = deployed_asset * shares_needed / deployed_shares;
    let min_asset_out = expected_asset * (BPS_DENOMINATOR - MAX_REBALANCE_SLIPPAGE_BPS) / BPS_DENOMINATOR;

    let vault_addr = storage::get_vault(env).ok_or(PoolError::VaultNotSet)?;
    let (asset_received, _principal) = redeem(env, &vault_addr, shares_needed, min_asset_out)?;
    storage::bump_instance_ttl(env);

    LiquidityAutoRebalanced {
        vault: vault_addr,
        shares_redeemed: shares_needed,
        asset_received,
    }
    .publish(env);

    Ok(asset_received)
}

// -----------------------------------------------------------------------
// r3 (2026-09-29): yield. Design rules, 2026-09-29:
//   1. Backers earn on MATURED money only (money in its wait is not in
//      `TotalBacked`, so the split below never reaches it).
//   2. Every owner takes their yield any time: no wait, no notice, no
//      pool-health check, and a staker keeps their stake.
//   3. No admin step: a yield payout harvests vault growth itself.
//   4. Staker and backer yield is set aside (`StakerYieldReserved`,
//      `BackerYieldReserved`): never deployed, never used for claims or
//      principal. The protocol share is pool cash, usable for claims until
//      the protocol withdraws it.
// Replaces the admin-only `extract_yield`.
// -----------------------------------------------------------------------

/// The one place yield is split. Called for every unit of growth that
/// comes back from the vault (`record_redeem` and `harvest`). Each side's
/// index moves by what it can represent exactly; rounding dust goes to the
/// protocol share, so the set-aside counters always equal what the indexes
/// owe (to within per-record floor rounding, which favours the pool).
pub(crate) fn credit_yield(env: &Env, amount: i128) {
    if amount <= 0 {
        return;
    }
    let total_staked = storage::get_total_staked(env);
    let total_backed = storage::get_total_backed(env);
    let capacity = total_staked + total_backed;

    let mut staker_share: i128 = 0;
    let mut backer_share: i128 = 0;
    if capacity > 0 {
        if total_staked > 0 {
            let part = amount * total_staked / capacity
                * crate::settings::get(env, crate::settings::SettingKey::StakerYieldBps)
                / YIELD_SPLIT_BPS_DENOMINATOR;
            let bump = part * YIELD_INDEX_PRECISION / total_staked;
            if bump > 0 {
                storage::set_yield_index(env, storage::get_yield_index(env) + bump);
                staker_share = bump * total_staked / YIELD_INDEX_PRECISION;
                storage::set_staker_yield_reserved(
                    env,
                    storage::get_staker_yield_reserved(env) + staker_share,
                );
            }
        }
        if total_backed > 0 {
            let part = amount * total_backed / capacity
                * crate::settings::get(env, crate::settings::SettingKey::BackerYieldBps)
                / YIELD_SPLIT_BPS_DENOMINATOR;
            let bump = part * YIELD_INDEX_PRECISION / total_backed;
            if bump > 0 {
                storage::set_backer_yield_index(env, storage::get_backer_yield_index(env) + bump);
                backer_share = bump * total_backed / YIELD_INDEX_PRECISION;
                storage::set_backer_yield_reserved(
                    env,
                    storage::get_backer_yield_reserved(env) + backer_share,
                );
            }
        }
    }
    let protocol_share = amount - staker_share - backer_share;
    storage::set_protocol_yield_balance(
        env,
        storage::get_protocol_yield_balance(env) + protocol_share,
    );
    storage::set_total_extracted_yield(env, storage::get_total_extracted_yield(env) + amount);
    storage::bump_instance_ttl(env);

    YieldCredited { yield_amount: amount, staker_share, backer_share, protocol_share }.publish(env);
}

/// Permissionless, best effort. Redeems only the vault's growth above book
/// value and credits it. Never fails and never changes book value: the
/// redeemed shares leave `TotalDeployedShares`, `TotalDeployedAsset` stays,
/// so the remaining shares still cover the full book value. Skipped while
/// paused. Returns the yield credited (0 if nothing to take or any vault
/// call fails).
pub fn harvest(env: &Env) -> i128 {
    if storage::is_paused(env) {
        return 0;
    }
    let Some(vault_addr) = storage::get_vault(env) else {
        return 0;
    };
    let deployed_shares = storage::get_total_deployed_shares(env);
    let deployed_asset = storage::get_total_deployed_asset(env);
    if deployed_shares <= 0 || deployed_asset <= 0 {
        return 0;
    }
    let vault = VaultClient::new(env, &vault_addr);
    let Ok(Ok(values)) = vault.try_get_asset_amounts_per_shares(&deployed_shares) else {
        return 0;
    };
    let value = values.get(0).unwrap_or(0);
    let growth = value - deployed_asset;
    if growth <= 0 || value <= 0 {
        return 0;
    }
    // CSO M2 (2026-09-29): growth limit per day since the last harvest, so a
    // share value pushed up for one transaction can book at most that much.
    // The clock starts at the first attempt; it moves only when a harvest
    // succeeds, so frequent callers cannot starve it.
    let now = env.ledger().timestamp();
    let last = storage::get_last_harvest_at(env);
    if last == 0 {
        storage::set_last_harvest_at(env, now);
        return 0;
    }
    let elapsed = now.saturating_sub(last) as i128;
    let limit = deployed_asset * HARVEST_MAX_GROWTH_BPS_PER_DAY * elapsed
        / (BPS_DENOMINATOR * SECONDS_PER_DAY as i128);
    let growth = growth.min(limit);
    // Floor: never redeem more shares than the growth is worth.
    let shares = growth * deployed_shares / value;
    if shares <= 0 {
        return 0;
    }
    let expected = value * shares / deployed_shares;
    let mut mins = Vec::new(env);
    mins.push_back(expected * (BPS_DENOMINATOR - HARVEST_SLIPPAGE_BPS) / BPS_DENOMINATOR);

    let liquid_before = liquid_balance(env);
    authorize_withdraw(env, &vault_addr, shares, &mins);
    if vault.try_withdraw(&shares, &mins, &env.current_contract_address()).is_err() {
        return 0;
    }
    let received = liquid_balance(env) - liquid_before;
    storage::set_total_deployed_shares(env, deployed_shares - shares);
    storage::set_last_harvest_at(env, now);
    credit_yield(env, received);
    received
}

/// Staker yield owed and not yet paid (additive index, see `StakeRecord`).
pub(crate) fn staker_yield_owed(env: &Env, record: &crate::types::StakeRecord) -> i128 {
    let delta = storage::get_yield_index(env) - record.yield_index_at_stake;
    if delta <= 0 || record.amount <= 0 {
        return 0;
    }
    record.amount * delta / YIELD_INDEX_PRECISION
}

/// Takes `owed` out of the staker set-aside. Returns what can actually be
/// paid: capped by the set-aside, which after a vault loss marks
/// `total_staked` down can hold less than the records' sum.
pub(crate) fn take_staker_yield(env: &Env, owed: i128) -> i128 {
    let reserved = storage::get_staker_yield_reserved(env);
    let paid = owed.min(reserved).max(0);
    storage::set_staker_yield_reserved(env, reserved - paid);
    paid
}

/// Same for backers.
pub(crate) fn take_backer_yield(env: &Env, owed: i128) -> i128 {
    let reserved = storage::get_backer_yield_reserved(env);
    let paid = owed.min(reserved).max(0);
    storage::set_backer_yield_reserved(env, reserved - paid);
    paid
}

/// Moves a forfeited stake's unpaid yield to the protocol share (design
/// rule 2026-09-29: a forfeited stake and its yield go to the pool).
pub(crate) fn forfeit_staker_yield(env: &Env, record: &mut crate::types::StakeRecord) {
    // Already out of `total_staked`: it has earned nothing since, and its
    // yield was settled when it left.
    if record.withdrawn {
        record.yield_index_at_stake = storage::get_yield_index(env);
        return;
    }
    let moved = take_staker_yield(env, staker_yield_owed(env, record));
    record.yield_index_at_stake = storage::get_yield_index(env);
    if moved > 0 {
        storage::set_protocol_yield_balance(
            env,
            storage::get_protocol_yield_balance(env) + moved,
        );
    }
}

/// What the pool holds (cash + deployed book value) above everything it owes:
/// stakes, counted and pending backing, set-aside yield, and the unpaid part
/// of open claims. The most `withdraw_yield` may ever send (code review W2).
pub(crate) fn protocol_surplus(env: &Env) -> i128 {
    let held = liquid_balance(env) + storage::get_total_deployed_asset(env);
    let owed = storage::get_total_staked(env)
        + storage::get_total_backed(env)
        + storage::get_total_backed_pending(env)
        + storage::get_yield_reserved(env)
        + storage::get_total_allocated(env);
    (held - owed).max(0)
}

/// V8 `withdrawYield` (`:860`), send the protocol's own realised yield
/// share to treasury. "The protocol's own money", it already sat
/// in `ProtocolYieldBalance` since `credit_yield` credited it there;
/// nothing new is realised or computed here, only paid out.
///
/// CHANGED 2026-09-18: gate switched from the residual `yield_balance()`
/// formula (removed, documented bug history) to the explicit
/// `ProtocolYieldBalance` counter. Still ports V8's double gate: the amount
/// must be within that balance AND within the real liquid balance, the
/// second check is what stops a treasury withdrawal from eating staker
/// principal that happens to be sitting liquid.
pub fn withdraw_yield(env: &Env, amount: i128) -> Result<(), PoolError> {
    let admin = storage::get_admin(env);
    admin.require_auth();

    if amount <= 0 {
        return Err(PoolError::AmountNotPositive);
    }
    let treasury = storage::get_treasury(env).ok_or(PoolError::TreasuryNotSet)?;

    if amount > storage::get_protocol_yield_balance(env) {
        return Err(PoolError::ExceedsYieldBalance);
    }
    // Code review W2 (design decision 2026-09-29): the protocol's share is pool
    // cash that claims may spend, and the counter above does not go down when
    // they do. So the protocol takes only what is left after every stake,
    // backing, set-aside yield and open claim is covered.
    if amount > protocol_surplus(env) {
        return Err(PoolError::ExceedsYieldBalance);
    }
    // r3: never touches staker/backer set-aside yield; pulls from the vault
    // if cash is short, like every other payment.
    pull_for_payment(env, amount)?;

    // NOTE 2026-09-18: total_extracted_yield is NOT incremented here.
    // credit_yield already counts the full realised amount the moment it
    // is redeemed from the vault, that is what "extracted" names. Adding
    // to it again here, now that crediting and the treasury payout are
    // two separate events instead of one atomic step, would double-count
    // every stroop that gets withdrawn (found as a real test failure while
    // building the split: 8M realised + 4M withdrawn read back as 12M).
    storage::set_protocol_yield_balance(
        env,
        storage::get_protocol_yield_balance(env) - amount,
    );
    storage::bump_instance_ttl(env);

    YieldWithdrawn {
        treasury: treasury.clone(),
        amount,
    }
    .publish(env);

    let token = TokenClient::new(env, &storage::get_asset_token(env));
    token.transfer(&env.current_contract_address(), &treasury, &amount);
    Ok(())
}
