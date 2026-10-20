//! Pre-audit hardening (2026-09-23): key-loss-safe governance. One test per
//! scenario from the mechanism review: a lost role, a stolen role, two lost
//! roles, and the pause that ends on its own.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::admin::PAUSE_MAX_SECONDS;
use crate::error::PoolError;
use crate::governance::{GOV_DELAY_SECONDS, RECOVERY_DELAY_SECONDS};
use crate::{GovChange, GovKind};

fn advance_secs(env: &soroban_sdk::Env, secs: u64) {
    advance_ledgers(env, (secs / SECONDS_PER_LEDGER) as u32);
}

fn vault_change(env: &soroban_sdk::Env) -> GovChange {
    GovChange::Vault(Address::generate(env))
}

// -- standard path: 2 of 3, 7 days --

#[test]
fn one_role_alone_can_never_pass_a_change() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_change(&s.admin, &vault_change(&env));
    advance_secs(&env, GOV_DELAY_SECONDS * 2);
    assert_eq!(s.client.try_execute_change(&GovKind::Vault), Err(Ok(PoolError::GovNotReady)));
}

#[test]
fn two_roles_pass_a_change_only_after_seven_days() {
    let env = new_env();
    let s = setup(&env);
    // r3: before any money a Vault change applies at once (instant setup);
    // the 7-day wait is the rule from the first stake on.
    staked_wallet(&env, &s);
    let change = vault_change(&env);
    s.client.propose_change(&s.admin, &change);
    s.client.approve_change(&s.guardian, &change);
    advance_secs(&env, GOV_DELAY_SECONDS - SECONDS_PER_LEDGER);
    assert_eq!(s.client.try_execute_change(&GovKind::Vault), Err(Ok(PoolError::GovNotReady)));
    advance_secs(&env, SECONDS_PER_LEDGER);
    s.client.execute_change(&GovKind::Vault);
    assert!(s.client.get_pending_change(&GovKind::Vault).is_none());
}

#[test]
fn approval_must_name_the_same_change() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_change(&s.admin, &vault_change(&env));
    assert_eq!(
        s.client.try_approve_change(&s.co_signer, &vault_change(&env)),
        Err(Ok(PoolError::GovChangeMismatch))
    );
}

#[test]
fn only_the_three_roles_can_act() {
    let env = new_env();
    let s = setup(&env);
    let stranger = Address::generate(&env);
    assert_eq!(
        s.client.try_propose_change(&stranger, &vault_change(&env)),
        Err(Ok(PoolError::GovCallerNotRole))
    );
    // The oracle is not a governance role either.
    assert_eq!(
        s.client.try_propose_change(&s.oracle, &vault_change(&env)),
        Err(Ok(PoolError::GovCallerNotRole))
    );
}

#[test]
fn one_pending_change_per_kind() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_change(&s.admin, &vault_change(&env));
    assert_eq!(
        s.client.try_propose_change(&s.co_signer, &vault_change(&env)),
        Err(Ok(PoolError::GovChangePending))
    );
}

// -- lost role: the other two replace it --

#[test]
fn lost_admin_is_replaced_by_the_other_two() {
    let env = new_env();
    let s = setup(&env);
    let new_admin = Address::generate(&env);
    let change = GovChange::Admin(new_admin.clone());
    // The admin never acts (its key is gone).
    s.client.propose_change(&s.co_signer, &change);
    s.client.approve_change(&s.guardian, &change);
    advance_secs(&env, GOV_DELAY_SECONDS);
    s.client.execute_change(&GovKind::Admin);
    assert_eq!(s.client.get_admin(), new_admin);
}

// -- stolen role: it can neither block nor clog its own removal --

#[test]
fn stolen_admin_cannot_propose_approve_or_cancel_its_own_replacement() {
    let env = new_env();
    let s = setup(&env);
    let change = GovChange::Admin(Address::generate(&env));
    assert_eq!(
        s.client.try_propose_change(&s.admin, &GovChange::Admin(Address::generate(&env))),
        Err(Ok(PoolError::GovTargetCannotApprove))
    );
    s.client.propose_change(&s.co_signer, &change);
    assert_eq!(
        s.client.try_approve_change(&s.admin, &change),
        Err(Ok(PoolError::GovTargetCannotApprove))
    );
    s.client.approve_change(&s.guardian, &change);
    assert_eq!(
        s.client.try_cancel_change(&s.admin, &GovKind::Admin),
        Err(Ok(PoolError::GovCannotCancel))
    );
    advance_secs(&env, GOV_DELAY_SECONDS);
    s.client.execute_change(&GovKind::Admin);
}

#[test]
fn one_role_cannot_cancel_alone_two_can() {
    let env = new_env();
    let s = setup(&env);
    s.client.propose_change(&s.admin, &vault_change(&env));
    // A lone (possibly stolen) role only casts a vote.
    assert!(!s.client.cancel_change(&s.guardian, &GovKind::Vault));
    assert!(s.client.get_pending_change(&GovKind::Vault).is_some());
    assert_eq!(
        s.client.try_cancel_change(&s.guardian, &GovKind::Vault),
        Err(Ok(PoolError::GovAlreadyApproved))
    );
    // A second role completes it.
    assert!(s.client.cancel_change(&s.co_signer, &GovKind::Vault));
    assert!(s.client.get_pending_change(&GovKind::Vault).is_none());
}

#[test]
fn replacing_a_role_voids_its_earlier_approvals() {
    let env = new_env();
    let s = setup(&env);
    let change = vault_change(&env);
    s.client.propose_change(&s.admin, &change);
    s.client.approve_change(&s.co_signer, &change);
    // The co-signer is replaced before the vault change executes.
    gov_apply(&env, &s, GovChange::CoSigner(Address::generate(&env)));
    assert_eq!(s.client.try_execute_change(&GovKind::Vault), Err(Ok(PoolError::GovNotReady)));
}

// -- two roles lost: 90-day recovery by the one left --

#[test]
fn last_role_recovers_after_ninety_days_if_nobody_objects() {
    let env = new_env();
    let s = setup(&env);
    // Admin and co-signer are both gone. The guardian acts alone.
    let new_admin = Address::generate(&env);
    s.client.propose_recovery(&s.guardian, &GovChange::Admin(new_admin.clone()));
    advance_secs(&env, RECOVERY_DELAY_SECONDS - SECONDS_PER_LEDGER);
    assert_eq!(s.client.try_execute_change(&GovKind::Admin), Err(Ok(PoolError::GovNotReady)));
    advance_secs(&env, SECONDS_PER_LEDGER);
    s.client.execute_change(&GovKind::Admin);
    assert_eq!(s.client.get_admin(), new_admin);

    // Two live roles again: the normal path replaces the lost co-signer.
    let new_co = Address::generate(&env);
    let change = GovChange::CoSigner(new_co.clone());
    s.client.propose_change(&new_admin, &change);
    s.client.approve_change(&s.guardian, &change);
    advance_secs(&env, GOV_DELAY_SECONDS);
    s.client.execute_change(&GovKind::CoSigner);
    assert_eq!(s.client.get_co_signer(), new_co);
}

#[test]
fn any_live_role_stops_a_recovery_alone_even_the_target() {
    let env = new_env();
    let s = setup(&env);
    // A thief holding the guardian tries to replace a live admin.
    s.client.propose_recovery(&s.guardian, &GovChange::Admin(Address::generate(&env)));
    assert!(s.client.cancel_change(&s.admin, &GovKind::Admin));
    assert!(s.client.get_pending_change(&GovKind::Admin).is_none());
}

#[test]
fn recovery_is_for_replacing_another_role_only() {
    let env = new_env();
    let s = setup(&env);
    assert_eq!(
        s.client.try_propose_recovery(&s.guardian, &vault_change(&env)),
        Err(Ok(PoolError::GovRecoveryRoleOnly))
    );
    assert_eq!(
        s.client.try_propose_recovery(&s.guardian, &GovChange::Guardian(Address::generate(&env))),
        Err(Ok(PoolError::GovRecoveryRoleOnly))
    );
    let change = GovChange::Admin(Address::generate(&env));
    s.client.propose_recovery(&s.guardian, &change);
    // A recovery is never fast-tracked by a second approval.
    assert_eq!(
        s.client.try_approve_change(&s.co_signer, &change),
        Err(Ok(PoolError::GovSoloProposal))
    );
}

// -- pause always ends --

#[test]
fn pause_ends_on_its_own_and_cannot_be_renewed_back_to_back() {
    let env = new_env();
    let s = setup(&env);
    s.client.pause();
    assert!(s.client.is_paused());
    advance_secs(&env, PAUSE_MAX_SECONDS - SECONDS_PER_LEDGER);
    assert!(s.client.is_paused());
    // 2026-10-16: no renewal while paused...
    assert_eq!(s.client.try_pause(), Err(Ok(PoolError::AlreadyPaused)));
    advance_secs(&env, SECONDS_PER_LEDGER);
    assert!(!s.client.is_paused());
    staked_wallet(&env, &s); // the pool works again with nobody acting
    // ...and none right after it ends: the gap (30 days) comes first.
    assert_eq!(s.client.try_pause(), Err(Ok(PoolError::PauseGapNotMet)));
    advance_secs(&env, 30 * crate::types::SECONDS_PER_DAY);
    assert_eq!(s.client.try_pause(), Err(Ok(PoolError::PauseGapNotMet)));
    advance_secs(&env, SECONDS_PER_LEDGER);
    s.client.pause();
    assert!(s.client.is_paused());
}

// -----------------------------------------------------------------------
// A new role key must sign its own installation
// -----------------------------------------------------------------------

/// Proposes and approves `change` with the right two roles, then waits out
/// the delay, so only the execute step is left.
fn ready_to_execute(env: &soroban_sdk::Env, s: &Setup<'_>, change: &GovChange, kind: GovKind) -> GovKind {
    let (admin, co, guardian) = (s.client.get_admin(), s.client.get_co_signer(), s.client.get_guardian());
    let (a, b) = match kind {
        GovKind::Admin => (co, guardian),
        GovKind::CoSigner => (admin, guardian),
        _ => (admin, co),
    };
    s.client.propose_change(&a, change);
    s.client.approve_change(&b, change);
    advance_secs(env, GOV_DELAY_SECONDS + 1);
    kind
}

#[test]
fn a_new_admin_is_installed_only_with_its_own_signature() {
    let env = new_env();
    let s = setup(&env);
    let new_admin = Address::generate(&env);
    let change = GovChange::Admin(new_admin.clone());
    let kind = ready_to_execute(&env, &s, &change, GovKind::Admin);

    // Without the new key's signature the change cannot be executed.
    env.set_auths(&[]);
    assert!(s.client.try_execute_change(&kind).is_err());
    assert_ne!(s.client.get_admin(), new_admin);

    // With it, the change goes through and the new key's signature was used.
    env.mock_all_auths();
    s.client.execute_change(&kind);
    assert!(env.auths().iter().any(|(who, _)| *who == new_admin));
    assert_eq!(s.client.get_admin(), new_admin);
}

#[test]
fn every_role_key_signs_its_own_installation() {
    for kind in [GovKind::CoSigner, GovKind::Guardian, GovKind::Oracle] {
        let env = new_env();
        let s = setup(&env);
        let key = Address::generate(&env);
        let change = match kind {
            GovKind::CoSigner => GovChange::CoSigner(key.clone()),
            GovKind::Guardian => GovChange::Guardian(key.clone()),
            _ => GovChange::Oracle(key.clone(), verifying_key_bytes(&env, &other_signing_key())),
        };
        let kind = ready_to_execute(&env, &s, &change, kind);
        env.set_auths(&[]);
        assert!(s.client.try_execute_change(&kind).is_err(), "{:?} installed without its signature", kind);
        env.mock_all_auths();
        s.client.execute_change(&kind);
        assert!(env.auths().iter().any(|(who, _)| *who == key), "{:?} key did not sign", kind);
    }
}

/// Stand-in for the covered-registry: records the writer the pool installs.
#[soroban_sdk::contract]
struct WriterProbe;

#[soroban_sdk::contractimpl]
impl WriterProbe {
    pub fn set_writer(env: soroban_sdk::Env, new_writer: Address) {
        env.storage().instance().set(&0u32, &new_writer);
    }
    pub fn writer(env: soroban_sdk::Env) -> Option<Address> {
        env.storage().instance().get(&0u32)
    }
}

/// A new registry writer key signs its own installation, like the role keys.
#[test]
fn a_new_registry_writer_is_installed_only_with_its_own_signature() {
    let env = new_env();
    let s = setup(&env);
    let registry = env.register(WriterProbe, ());
    let probe = WriterProbeClient::new(&env, &registry);
    let writer = Address::generate(&env);
    let change = GovChange::RegistryWriter(registry.clone(), writer.clone());
    let kind = ready_to_execute(&env, &s, &change, GovKind::RegistryWriter);

    env.set_auths(&[]);
    assert!(s.client.try_execute_change(&kind).is_err());
    assert_eq!(probe.writer(), None);

    env.mock_all_auths();
    s.client.execute_change(&kind);
    assert!(env.auths().iter().any(|(who, _)| *who == writer));
    assert_eq!(probe.writer(), Some(writer));
}
