#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::error::PoolError;
use crate::types::{APPROVE_WINDOW_LEDGERS, COOLDOWN_LEDGERS, TIME_GATE_LEDGERS, VESTING_LEDGERS};

// -----------------------------------------------------------------------
// Constructor tests.
//
// REWRITTEN 2026-08-17 (7a audit, Finding 3). Configuration moved from a
// separate `initialize` entrypoint to `__constructor`, which closes the
// deploy->init front-running window. Consequence for testing: a failing
// constructor **aborts the deployment**, so `env.register` panics rather
// than returning a `Result`: there is no `try_register`, and no
// `try_initialize` client method exists any more.
//
// These therefore assert on the panic, pinned to the specific contract error
// code (`Error(Contract, #N)`) so they still fail if the WRONG validation
// fires: the codes are `PoolError`'s public ABI discriminants:
// OracleEqualsCoSigner = 11, CoSignerEqualsAdmin = 12, PoolCapNotPositive = 3.
// Weaker than the previous `assert_eq!(..., Err(Ok(PoolError::X)))` in form,
// but it exercises exactly what a real deployer hits, which the old test did
// not. `initialize_guard_rejects_second_call` below keeps a typed assertion on
// the reinit guard by calling the internal helper directly.
// -----------------------------------------------------------------------

const TIER_C: u32 = 3;

#[test]
fn constructor_sets_all_fields() {
    let env = new_env();
    let s = setup(&env);
    // Successful construction is the assertion (setup would have panicked);
    // roundtrip a stake to confirm the pool cap actually took effect.
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &MID_STAKE, &ben);
}

/// The reinit guard is unreachable through the ABI now (`__constructor` runs
/// only at creation), so this drives `admin::initialize` directly inside the
/// already-constructed contract's storage context. That keeps a typed-error
/// assertion on the guard, and pins that it checks BEFORE writing anything.
#[test]
fn initialize_guard_rejects_second_call() {
    let env = new_env();
    let s = setup(&env);
    let pubkey = verifying_key_bytes(&env, &oracle_signing_key());
    let result = env.as_contract(&s.contract_id, || {
        crate::admin::initialize(
            &env,
            &s.admin,
            &s.oracle,
            &pubkey,
            &s.co_signer,
            &s.guardian,
            &s.token_id,
            POOL_CAP,
        )
    });
    assert_eq!(result, Err(PoolError::AlreadyInitialized));
}

#[test]
#[should_panic(expected = "#11")] // PoolError::OracleEqualsCoSigner
fn constructor_oracle_equals_cosigner_aborts_deploy() {
    let env = new_env();
    let admin = Address::generate(&env);
    let same = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();
    env.register(
        crate::ProtectionPool,
        (
            admin,
            same.clone(),
            verifying_key_bytes(&env, &oracle_signing_key()),
            same,
            Address::generate(&env),
            token_id,
            POOL_CAP,
        ),
    );
}

#[test]
#[should_panic(expected = "#12")] // PoolError::CoSignerEqualsAdmin
fn constructor_cosigner_equals_admin_aborts_deploy() {
    let env = new_env();
    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();
    env.register(
        crate::ProtectionPool,
        (
            admin.clone(),
            oracle,
            verifying_key_bytes(&env, &oracle_signing_key()),
            admin,
            Address::generate(&env),
            token_id,
            POOL_CAP,
        ),
    );
}

#[test]
#[should_panic(expected = "#3")] // PoolError::PoolCapNotPositive
fn constructor_zero_pool_cap_aborts_deploy() {
    let env = new_env();
    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let co_signer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();
    env.register(
        crate::ProtectionPool,
        (
            admin,
            oracle,
            verifying_key_bytes(&env, &oracle_signing_key()),
            co_signer,
            Address::generate(&env),
            token_id,
            0_i128,
        ),
    );
}

#[test]
fn oracle_changes_through_governance() {
    let env = new_env();
    let s = setup(&env);
    let new_oracle = Address::generate(&env);
    let key = verifying_key_bytes(&env, &s.oracle_key);
    gov_apply(&env, &s, crate::GovChange::Oracle(new_oracle.clone(), key));
    assert_eq!(s.client.get_oracle(), new_oracle);
    // Confirmed indirectly: submit_claim as the NEW oracle now works.
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_days(&env, 91);
    submit_claim_signed(&env, &s, &new_oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn oracle_equal_to_a_role_is_refused() {
    let env = new_env();
    let s = setup(&env);
    let key = verifying_key_bytes(&env, &s.oracle_key);
    let result = s.client.try_propose_change(&s.admin, &crate::GovChange::Oracle(s.co_signer.clone(), key));
    assert_eq!(result, Err(Ok(PoolError::GovRoleNotDistinct)));
}

#[test]
fn co_signer_changes_through_governance() {
    let env = new_env();
    let s = setup(&env);
    let new_cs = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::CoSigner(new_cs.clone()));
    assert_eq!(s.client.get_co_signer(), new_cs);
}

#[test]
fn co_signer_equal_to_oracle_is_refused() {
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_propose_change(&s.admin, &crate::GovChange::CoSigner(s.oracle.clone()));
    assert_eq!(result, Err(Ok(PoolError::GovRoleNotDistinct)));
}

#[test]
fn set_pool_cap_increases() {
    let env = new_env();
    let s = setup(&env);
    s.client.set_pool_cap(&(POOL_CAP * 2));
    // New bounds take effect immediately, a stake above the old MAX_STAKE
    // now succeeds.
    let bigger = MAX_STAKE + 1;
    let staker = new_funded_address(&env, &s, bigger);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &bigger, &ben);
}

#[test]
fn set_pool_cap_zero_panics() {
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_set_pool_cap(&0);
    assert_eq!(result, Err(Ok(PoolError::PoolCapNotPositive)));
}

#[test]
fn set_pool_cap_below_total_staked_panics() {
    let env = new_env();
    let s = setup(&env);
    staked_wallet(&env, &s);
    let result = s.client.try_set_pool_cap(&(MID_STAKE - 1));
    assert_eq!(result, Err(Ok(PoolError::PoolCapBelowTotalStaked)));
}

#[test]
fn admin_changes_through_governance() {
    let env = new_env();
    let s = setup(&env);
    let new_admin = Address::generate(&env);
    gov_apply(&env, &s, crate::GovChange::Admin(new_admin.clone()));
    assert_eq!(s.client.get_admin(), new_admin);
}

#[test]
fn admin_equal_to_co_signer_is_refused() {
    let env = new_env();
    let s = setup(&env);
    let result = s.client.try_propose_change(&s.guardian, &crate::GovChange::Admin(s.co_signer.clone()));
    assert_eq!(result, Err(Ok(PoolError::GovRoleNotDistinct)));
}

#[test]
#[should_panic(expected = "#125")] // PoolError::GovRoleNotDistinct
fn constructor_guardian_equal_to_admin_aborts_deploy() {
    let env = new_env();
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    env.register(
        crate::ProtectionPool,
        (
            admin.clone(),
            Address::generate(&env),
            verifying_key_bytes(&env, &oracle_signing_key()),
            Address::generate(&env),
            admin,
            sac.address(),
            POOL_CAP,
        ),
    );
}

#[test]
fn pause_blocks_stake() {
    let env = new_env();
    let s = setup(&env);
    s.client.pause();
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    let result = s.client.try_stake(&staker, &MID_STAKE, &ben);
    assert_eq!(result, Err(Ok(PoolError::Paused)));
}

#[test]
fn unpause_restores_stake() {
    let env = new_env();
    let s = setup(&env);
    s.client.pause();
    s.client.unpause();
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);
    s.client.stake(&staker, &MID_STAKE, &ben);
}

#[test]
fn pause_blocks_withdraw() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.pause();
    let result = s.client.try_withdraw(&staker, &ben);
    assert_eq!(result, Err(Ok(PoolError::Paused)));
}

#[test]
fn suspend_stake_blocks_nothing_about_withdrawal() {
    // suspendStake blocks payout eligibility, NOT principal withdrawal,
    // this is the exact V8 semantic this test locks in.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.suspend_stake(&staker);
    s.client.withdraw(&staker, &ben);
}

#[test]
fn suspend_stake_blocks_claim_submission() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    s.client.suspend_stake(&staker);
    let result = try_submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
    assert_eq!(result, Err(Ok(PoolError::StakeSuspended)));
}

#[test]
fn unsuspend_stake_restores_claim_eligibility() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    s.client.suspend_stake(&staker);
    s.client.unsuspend_stake(&staker, &None);
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
}

#[test]
fn suspend_stake_on_nonexistent_stake_panics() {
    let env = new_env();
    let s = setup(&env);
    let random = Address::generate(&env);
    let result = s.client.try_suspend_stake(&random);
    assert_eq!(result, Err(Ok(PoolError::NoStake)));
}

/// Mutation-testing gap fix (2026-07-22 re-run, 485 mutants, 10 missed).
/// Kills admin.rs:164 (`!=`->`==`, which would return early even on a
/// MATCHING wallet and skip the reset entirely), admin.rs:169 (deleted
/// AwaitingApproval match arm), and admin.rs:170 x2 (`+`->`-`/`+`->`*` on
/// the deadline arithmetic), the exact-value assertion below catches all
/// three, since any of them leaves `approve_deadline_ledger` at something
/// other than `now + APPROVE_WINDOW_LEDGERS`.
#[test]
fn unsuspend_stake_resets_rule_a_deadline_for_awaiting_approval() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.suspend_stake(&staker);
    advance_ledgers(&env, APPROVE_WINDOW_LEDGERS / 2); // partway into the approve window
    s.client.unsuspend_stake(&staker, &Some(claim_id.clone()));
    let claim = s.client.get_claim(&claim_id).unwrap();
    let expected = env.ledger().sequence() + crate::types::APPROVE_WINDOW_LEDGERS;
    assert_eq!(claim.approve_deadline_ledger, expected);
}

/// Kills admin.rs:173 (deleted Active match arm), same wallet-ownership
/// check as the AwaitingApproval test above also runs through this path,
/// but the Active branch's clock reset is independently gapped: without
/// it, `last_collected_ledger` would stay at its pre-suspend value instead
/// of resetting to the unsuspend moment.
#[test]
fn unsuspend_stake_resets_rule_b_clock_for_active_claim() {
    // Ledger advances derived from the real gate/
    // cooldown/vesting constants, and kept to a fraction of vesting so the
    // claim stays Active (not auto-Completed) through the suspend/unsuspend
    // cycle -- the whole point of this test is exercising the Active branch.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    advance_ledgers(&env, TIME_GATE_LEDGERS);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
    s.client.approve_claim(&claim_id); // -> Active, cooldown starts
    advance_ledgers(&env, COOLDOWN_LEDGERS); // cooldown passes
    advance_ledgers(&env, VESTING_LEDGERS / 4); // partway into vesting/collection
    s.client.claim_stream(&claim_id, &ben);
    s.client.suspend_stake(&staker);
    advance_ledgers(&env, VESTING_LEDGERS / 4);
    s.client.unsuspend_stake(&staker, &Some(claim_id.clone()));
    let claim = s.client.get_claim(&claim_id).unwrap();
    assert_eq!(claim.last_collected_ledger, env.ledger().sequence());
}

#[test]
fn suspend_stake_after_withdraw_panics() {
    // Reaches PoolError::NoStake (amount<=0), not AlreadyWithdrawn, same
    // check-ordering note as set_beneficiary_after_withdraw_panics in
    // stake_tests.rs.
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);
    s.client.withdraw(&staker, &ben);
    let result = s.client.try_suspend_stake(&staker);
    assert_eq!(result, Err(Ok(PoolError::NoStake)));
}
