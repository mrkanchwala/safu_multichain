//! Pre-audit hardening (2026-09-23): key-loss-safe governance.
//!
//! The pool is never upgraded, so it must survive losing or leaking any key
//! for its whole life. Design decision, after a scenario sweep (mechanism
//! review 2026-09-23):
//!
//! - THREE roles: admin, co-signer, guardian. Each is meant to be a 2-of-3
//!   native Stellar multisig account, so a single lost or leaked key never
//!   reaches this contract at all (`require_auth` on a Stellar account checks
//!   its own signers against its medium threshold).
//! - Standard path: any role proposes, a SECOND role approves the same
//!   change, then it waits `GOV_DELAY_SECONDS` (7 days) in public, then
//!   anyone executes. Cancelling also takes two roles (2026-09-23),
//!   so a lone thief cannot keep cancelling honest changes. The role a
//!   change replaces never counts, cannot propose it and cannot cancel it,
//!   so a stolen role can neither block nor clog its own removal.
//! - Last-resort recovery: if two roles are gone, the one left can propose a
//!   role replacement alone. It waits `RECOVERY_DELAY_SECONDS` (90 days), and
//!   any other role still working can cancel it, including the one it
//!   replaces. So it only lands when nobody else is there to object. (One
//!   role, not two, deliberately: with 2-of-3 here, one honest role could
//!   not stop a thief's recovery when the third role is lost; the scenario
//!   sweep put that at ~20x the takeover risk.)
//!
//! Two roles alone could never settle a dispute (which of them is the
//! thief?); a third role breaks the tie. What cannot be recovered: all three
//! roles lost, or one honest role against one thief, where each cancels the
//! other. Stakers can still withdraw and collect claims in both cases, and a
//! pause expires on its own (`admin.rs`), so no money is frozen forever.
//!
//! Changes handled here (everything that grants power or moves where money
//! goes): the three roles, the oracle (policy address + attestation key
//! together), the yield vault, the treasury, the vault deploy ceiling, and
//! the covered-wallet registry's writer key.
//! Only role replacements can use the recovery path.

use soroban_sdk::{
    contractevent, contracttype, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};

use crate::error::PoolError;
use crate::storage;
use crate::types::{MAX_DEPLOY_BPS, SECONDS_PER_DAY};

pub const GOV_DELAY_SECONDS: u64 = 7 * SECONDS_PER_DAY;
pub const RECOVERY_DELAY_SECONDS: u64 = 90 * SECONDS_PER_DAY;

/// One pending slot per kind. Discriminants are public ABI: append only.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovKind {
    Admin = 0,
    CoSigner = 1,
    Guardian = 2,
    Oracle = 3,
    Vault = 4,
    Treasury = 5,
    DeployBps = 6,
    /// The covered-wallet registry's writer key (the registry obeys only
    /// this pool, so it inherits the same governance and recovery).
    RegistryWriter = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovChange {
    Admin(Address),
    CoSigner(Address),
    Guardian(Address),
    /// Policy address and Ed25519 attestation key move together, so there
    /// is never a window where they belong to different oracles.
    Oracle(Address, BytesN<32>),
    Vault(Address),
    Treasury(Address),
    DeployBps(i128),
    /// (registry contract, new writer)
    RegistryWriter(Address, Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovProposal {
    pub change: GovChange,
    pub proposer: Address,
    /// Last-resort recovery: one role, 90 days, cancellable by any other.
    pub solo: bool,
    /// Each role's approval, stored as the address that gave it. Counted only
    /// while it still equals that role's CURRENT address, so a rotation
    /// voids stale approvals.
    pub admin_approver: Option<Address>,
    pub co_signer_approver: Option<Address>,
    pub guardian_approver: Option<Address>,
    /// Cancel votes on the standard path (two live votes cancel), same
    /// rules as approvals.
    pub admin_canceller: Option<Address>,
    pub co_signer_canceller: Option<Address>,
    pub guardian_canceller: Option<Address>,
    /// Earliest execution; 0 until two roles have approved (standard path).
    pub eta: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Admin,
    CoSigner,
    Guardian,
}

// -----------------------------------------------------------------------
// Events: every step is public.
// -----------------------------------------------------------------------

#[contractevent]
pub struct GovProposed {
    #[topic]
    pub kind: GovKind,
    pub change: GovChange,
    pub proposer: Address,
    pub solo: bool,
    pub eta: u64,
}

#[contractevent]
pub struct GovApproved {
    #[topic]
    pub kind: GovKind,
    pub approver: Address,
    pub eta: u64,
}

#[contractevent]
pub struct GovCancelVote {
    #[topic]
    pub kind: GovKind,
    pub by: Address,
}

#[contractevent]
pub struct GovCancelled {
    #[topic]
    pub kind: GovKind,
    pub by: Address,
}

#[contractevent]
pub struct GovExecuted {
    #[topic]
    pub kind: GovKind,
    pub change: GovChange,
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn kind_of(change: &GovChange) -> GovKind {
    match change {
        GovChange::Admin(_) => GovKind::Admin,
        GovChange::CoSigner(_) => GovKind::CoSigner,
        GovChange::Guardian(_) => GovKind::Guardian,
        GovChange::Oracle(_, _) => GovKind::Oracle,
        GovChange::Vault(_) => GovKind::Vault,
        GovChange::Treasury(_) => GovKind::Treasury,
        GovChange::DeployBps(_) => GovKind::DeployBps,
        GovChange::RegistryWriter(_, _) => GovKind::RegistryWriter,
    }
}

/// The role a change replaces, if any. That role has no say in it.
fn target_of(kind: GovKind) -> Option<Role> {
    match kind {
        GovKind::Admin => Some(Role::Admin),
        GovKind::CoSigner => Some(Role::CoSigner),
        GovKind::Guardian => Some(Role::Guardian),
        _ => None,
    }
}

fn role_address(env: &Env, role: Role) -> Address {
    match role {
        Role::Admin => storage::get_admin(env),
        Role::CoSigner => storage::get_co_signer(env),
        Role::Guardian => storage::get_guardian(env),
    }
}

fn role_of(env: &Env, caller: &Address) -> Result<Role, PoolError> {
    for role in [Role::Admin, Role::CoSigner, Role::Guardian] {
        if &role_address(env, role) == caller {
            return Ok(role);
        }
    }
    Err(PoolError::GovCallerNotRole)
}

fn approver_slot(p: &mut GovProposal, role: Role) -> &mut Option<Address> {
    match role {
        Role::Admin => &mut p.admin_approver,
        Role::CoSigner => &mut p.co_signer_approver,
        Role::Guardian => &mut p.guardian_approver,
    }
}

fn canceller_slot(p: &mut GovProposal, role: Role) -> &mut Option<Address> {
    match role {
        Role::Admin => &mut p.admin_canceller,
        Role::CoSigner => &mut p.co_signer_canceller,
        Role::Guardian => &mut p.guardian_canceller,
    }
}

/// Approvals that still count: from a role's CURRENT address, never from the
/// role being replaced.
fn live_approvals(env: &Env, p: &GovProposal) -> u32 {
    live_votes(env, p, [&p.admin_approver, &p.co_signer_approver, &p.guardian_approver])
}

fn live_cancels(env: &Env, p: &GovProposal) -> u32 {
    live_votes(env, p, [&p.admin_canceller, &p.co_signer_canceller, &p.guardian_canceller])
}

fn live_votes(env: &Env, p: &GovProposal, slots: [&Option<Address>; 3]) -> u32 {
    let target = target_of(kind_of(&p.change));
    let mut n = 0;
    for (role, slot) in [Role::Admin, Role::CoSigner, Role::Guardian].into_iter().zip(slots) {
        if Some(role) != target && slot.as_ref() == Some(&role_address(env, role)) {
            n += 1;
        }
    }
    n
}

/// Admin, co-signer, guardian and oracle must stay four different
/// addresses, so no single account ever holds two votes.
pub fn check_roles_distinct(
    admin: &Address,
    co_signer: &Address,
    guardian: &Address,
    oracle: &Address,
) -> Result<(), PoolError> {
    let all = [admin, co_signer, guardian, oracle];
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            if all[i] == all[j] {
                return Err(PoolError::GovRoleNotDistinct);
            }
        }
    }
    Ok(())
}

/// Checked when proposed AND again when executed, against live state.
fn validate(env: &Env, change: &GovChange) -> Result<(), PoolError> {
    let (mut admin, mut co, mut guardian, mut oracle) = (
        storage::get_admin(env),
        storage::get_co_signer(env),
        storage::get_guardian(env),
        storage::get_oracle(env),
    );
    match change {
        GovChange::Admin(a) => admin = a.clone(),
        GovChange::CoSigner(c) => co = c.clone(),
        GovChange::Guardian(g) => guardian = g.clone(),
        GovChange::Oracle(o, _) => oracle = o.clone(),
        GovChange::Vault(_) => {
            if storage::get_total_deployed_shares(env) > 0 {
                return Err(PoolError::VaultChangeWhileDeployed);
            }
            return Ok(());
        }
        GovChange::Treasury(_) | GovChange::RegistryWriter(_, _) => return Ok(()),
        GovChange::DeployBps(bps) => {
            if *bps < 0 {
                return Err(PoolError::AmountNotPositive);
            }
            if *bps > MAX_DEPLOY_BPS {
                return Err(PoolError::DeployBpsTooHigh);
            }
            return Ok(());
        }
    }
    check_roles_distinct(&admin, &co, &guardian, &oracle)
}

fn apply(env: &Env, change: &GovChange) {
    match change {
        GovChange::Admin(a) => storage::set_admin(env, a),
        GovChange::CoSigner(c) => storage::set_co_signer(env, c),
        GovChange::Guardian(g) => storage::set_guardian(env, g),
        GovChange::Oracle(o, key) => {
            storage::set_oracle(env, o);
            storage::set_oracle_pubkey(env, key);
        }
        GovChange::Vault(v) => storage::set_vault(env, v),
        GovChange::Treasury(t) => storage::set_treasury(env, t),
        GovChange::DeployBps(bps) => storage::set_deploy_bps(env, *bps),
        GovChange::RegistryWriter(registry, writer) => {
            // The registry checks that THIS pool is the caller.
            let args: Vec<Val> = vec![env, writer.into_val(env)];
            env.invoke_contract::<()>(registry, &Symbol::new(env, "set_writer"), args);
        }
    }
}

// -----------------------------------------------------------------------
// Standard path: propose -> second role approves -> 7 days -> execute
// -----------------------------------------------------------------------

/// Any role proposes; its own approval is recorded unless the change
/// replaces that role. One pending change per kind: cancel it first to
/// propose a different one.
pub fn propose_change(env: &Env, caller: &Address, change: GovChange) -> Result<(), PoolError> {
    caller.require_auth();
    let role = role_of(env, caller)?;
    let kind = kind_of(&change);
    // A role never proposes its own replacement (stops a thief clogging the
    // one slot that would remove it).
    if target_of(kind) == Some(role) {
        return Err(PoolError::GovTargetCannotApprove);
    }
    if storage::get_gov_pending(env, kind).is_some() {
        return Err(PoolError::GovChangePending);
    }
    validate(env, &change)?;

    let mut p = GovProposal {
        change: change.clone(),
        proposer: caller.clone(),
        solo: false,
        admin_approver: None,
        co_signer_approver: None,
        guardian_approver: None,
        admin_canceller: None,
        co_signer_canceller: None,
        guardian_canceller: None,
        eta: 0,
    };
    *approver_slot(&mut p, role) = Some(caller.clone());
    storage::set_gov_pending(env, kind, &p);
    storage::bump_instance_ttl(env);
    GovProposed { kind, change, proposer: caller.clone(), solo: false, eta: 0 }.publish(env);
    Ok(())
}

/// A second role approves the SAME change. On the second live approval the
/// 7-day public clock starts.
pub fn approve_change(env: &Env, caller: &Address, change: GovChange) -> Result<(), PoolError> {
    caller.require_auth();
    let role = role_of(env, caller)?;
    let kind = kind_of(&change);
    let mut p = storage::get_gov_pending(env, kind).ok_or(PoolError::GovNoPendingChange)?;
    if p.change != change {
        return Err(PoolError::GovChangeMismatch);
    }
    if p.solo {
        return Err(PoolError::GovSoloProposal);
    }
    if target_of(kind) == Some(role) {
        return Err(PoolError::GovTargetCannotApprove);
    }
    if approver_slot(&mut p, role).as_ref() == Some(caller) {
        return Err(PoolError::GovAlreadyApproved);
    }
    *approver_slot(&mut p, role) = Some(caller.clone());
    if p.eta == 0 && live_approvals(env, &p) >= 2 {
        let now = env.ledger().timestamp();
        // r3 (2026-09-29, design decision): before any money has ever entered the
        // pool, vault / treasury / deploy ceiling need no wait: nothing is
        // at risk and the wait protects nobody. `EverFunded` is set on the
        // first stake or backing and never cleared, so a pool that later
        // empties does not reopen this. Still needs two roles, as always.
        // `max(1)`: 0 means "not approved yet" in `execute_change`.
        p.eta = if instant_before_funding(env, kind) {
            now.max(1)
        } else {
            now + GOV_DELAY_SECONDS
        };
    }
    storage::set_gov_pending(env, kind, &p);
    storage::bump_instance_ttl(env);
    GovApproved { kind, approver: caller.clone(), eta: p.eta }.publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// Last-resort recovery: one role, 90 days, any other role can cancel
// -----------------------------------------------------------------------

pub fn propose_recovery(env: &Env, caller: &Address, change: GovChange) -> Result<(), PoolError> {
    caller.require_auth();
    let role = role_of(env, caller)?;
    let kind = kind_of(&change);
    match target_of(kind) {
        None => return Err(PoolError::GovRecoveryRoleOnly),
        Some(t) if t == role => return Err(PoolError::GovRecoveryRoleOnly),
        _ => {}
    }
    if storage::get_gov_pending(env, kind).is_some() {
        return Err(PoolError::GovChangePending);
    }
    validate(env, &change)?;

    let eta = env.ledger().timestamp() + RECOVERY_DELAY_SECONDS;
    let mut p = GovProposal {
        change: change.clone(),
        proposer: caller.clone(),
        solo: true,
        admin_approver: None,
        co_signer_approver: None,
        guardian_approver: None,
        admin_canceller: None,
        co_signer_canceller: None,
        guardian_canceller: None,
        eta,
    };
    *approver_slot(&mut p, role) = Some(caller.clone());
    storage::set_gov_pending(env, kind, &p);
    storage::bump_instance_ttl(env);
    GovProposed { kind, change, proposer: caller.clone(), solo: true, eta }.publish(env);
    Ok(())
}

// -----------------------------------------------------------------------
// Cancel and execute
// -----------------------------------------------------------------------

/// Standard change: two roles must vote to cancel, never counting the role
/// being replaced (a thief can't block its own removal, and one thief alone
/// can't cancel anything). Returns true once cancelled, false while the
/// first vote waits for a second.
/// Recovery: any single role cancels at once, including the one it would
/// replace: a recovery only lands when nobody objects.
pub fn cancel_change(env: &Env, caller: &Address, kind: GovKind) -> Result<bool, PoolError> {
    caller.require_auth();
    let role = role_of(env, caller)?;
    let mut p = storage::get_gov_pending(env, kind).ok_or(PoolError::GovNoPendingChange)?;
    if !p.solo {
        if target_of(kind) == Some(role) {
            return Err(PoolError::GovCannotCancel);
        }
        if canceller_slot(&mut p, role).as_ref() == Some(caller) {
            return Err(PoolError::GovAlreadyApproved);
        }
        *canceller_slot(&mut p, role) = Some(caller.clone());
        if live_cancels(env, &p) < 2 {
            storage::set_gov_pending(env, kind, &p);
            storage::bump_instance_ttl(env);
            GovCancelVote { kind, by: caller.clone() }.publish(env);
            return Ok(false);
        }
    }
    storage::remove_gov_pending(env, kind);
    storage::bump_instance_ttl(env);
    GovCancelled { kind, by: caller.clone() }.publish(env);
    Ok(true)
}

/// Anyone, once the wait has passed. Approvals and the change itself are
/// re-checked against live state.
pub fn execute_change(env: &Env, kind: GovKind) -> Result<(), PoolError> {
    let p = storage::get_gov_pending(env, kind).ok_or(PoolError::GovNoPendingChange)?;
    if p.eta == 0 || env.ledger().timestamp() < p.eta {
        return Err(PoolError::GovNotReady);
    }
    if p.solo {
        // The proposer must still hold its role.
        if role_of(env, &p.proposer).is_err() {
            return Err(PoolError::GovNotReady);
        }
    } else if live_approvals(env, &p) < 2 {
        return Err(PoolError::GovNotReady);
    }
    validate(env, &p.change)?;
    apply(env, &p.change);
    storage::remove_gov_pending(env, kind);
    storage::bump_instance_ttl(env);
    GovExecuted { kind, change: p.change }.publish(env);
    Ok(())
}

pub fn get_pending(env: &Env, kind: GovKind) -> Option<GovProposal> {
    storage::get_gov_pending(env, kind)
}

/// r3: the setup kinds that apply at once while the pool has never held money.
const INSTANT_SETUP_KINDS: [GovKind; 3] = [GovKind::Vault, GovKind::Treasury, GovKind::DeployBps];

fn instant_before_funding(env: &Env, kind: GovKind) -> bool {
    !storage::is_ever_funded(env) && INSTANT_SETUP_KINDS.contains(&kind)
}

/// Called once, when the pool is first funded (code review B1, 2026-09-29).
/// A setup change approved while the pool was empty but not yet executed
/// would otherwise stay executable at once after money arrives. Its wait
/// starts now instead. Only ever pushes an `eta` later, never earlier.
pub(crate) fn end_instant_setup(env: &Env) {
    let later = env.ledger().timestamp() + GOV_DELAY_SECONDS;
    for kind in INSTANT_SETUP_KINDS {
        if let Some(mut p) = storage::get_gov_pending(env, kind) {
            if p.eta != 0 && p.eta < later {
                p.eta = later;
                storage::set_gov_pending(env, kind, &p);
            }
        }
    }
}
