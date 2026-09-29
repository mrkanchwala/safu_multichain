#![cfg(test)]

//! Resource-cost profiling (Task #13, 2026-07-14), measures CPU
//! instruction + memory cost per hot-path entrypoint via soroban-sdk's
//! built-in budget tracker (`env.cost_estimate().budget()`). Entirely
//! local, no deploy: this is the same in-process test host `cargo test`
//! already uses, just reading its resource accounting instead of
//! ignoring it.
//!
//! Caveat, straight from the soroban-sdk docs: CPU/memory costs are
//! LOWER BOUNDS here: native Rust execution in the test host
//! underestimates cost relative to the real compiled WASM running in a
//! WASM VM. These numbers are useful for catching regressions (a change
//! that suddenly costs 10x more) and rough hot-path comparison, not as
//! literal mainnet fee predictions. Getting WASM-accurate numbers
//! requires loading the actual .wasm bytes into the test host instead of
//! registering the native Rust type, flagged as a follow-up, not done
//! here to avoid another round of unverified API guessing this session.
//!
//! Run with `cargo test --package protection-pool profiling -- --nocapture`
//! to see the printed budget breakdown per operation.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;

const TIER_C: u32 = 3;

/// Regression ceiling for a single hot-path call, in CPU instructions. It is a
/// deliberate tripwire for a change that suddenly costs far more, not a
/// network limit or a fee prediction (see the caveat above).
const MAX_CPU_INSTRUCTIONS: u64 = 50_000_000;

fn print_budget(env: &soroban_sdk::Env, label: &str) {
    let budget = env.cost_estimate().budget();
    std::println!(
        "[profiling] {label}: cpu_instructions={} memory_bytes={}",
        budget.cpu_instruction_cost(),
        budget.memory_bytes_cost()
    );
}

#[test]
fn profile_stake() {
    let env = new_env();
    let s = setup(&env);
    let staker = new_funded_address(&env, &s, MID_STAKE);
    let ben = Address::generate(&env);

    env.cost_estimate().budget().reset_default();
    s.client.stake(&staker, &MID_STAKE, &ben);
    print_budget(&env, "stake");

    // Regression guard, not a real cost prediction (see module doc),
    // catches an accidental 10x+ blowup in a hot path, not meant to be a
    // tight bound.
    assert!(env.cost_estimate().budget().cpu_instruction_cost() < MAX_CPU_INSTRUCTIONS);
}

#[test]
fn profile_withdraw() {
    let env = new_env();
    let s = setup(&env);
    let (staker, ben) = staked_wallet(&env, &s);

    env.cost_estimate().budget().reset_default();
    s.client.withdraw(&staker, &ben);
    print_budget(&env, "withdraw");

    assert!(env.cost_estimate().budget().cpu_instruction_cost() < MAX_CPU_INSTRUCTIONS);
}

#[test]
fn profile_submit_claim() {
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    // Gate met: exercises the AwaitingApproval branch (2026-07-22:
    // no longer immediate activation, see approve_claim for that cost).
    advance_days(&env, 90);

    env.cost_estimate().budget().reset_default();
    submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &STROOPS_PER_UNIT,
        &TIER_C,
        &now_ts(&env),
    );
    print_budget(&env, "submit_claim (gate met, lands in AwaitingApproval)");

    assert!(env.cost_estimate().budget().cpu_instruction_cost() < MAX_CPU_INSTRUCTIONS);
}

#[test]
fn profile_claim_stream() {
    let env = new_env();
    let s = setup(&env);
    let entitlement = STROOPS_PER_UNIT;
    let (staker, ben) = staked_wallet(&env, &s);
    advance_days(&env, 90);
    let claim_id = submit_claim_signed(&env, &s, &s.oracle,
        &staker,
        &tx_hash(&env, 1),
        &entitlement,
        &TIER_C,
        &now_ts(&env),
    );
    // CHANGED 2026-07-22: gate-met no longer auto-activates, lands in
    // AwaitingApproval, needs an explicit approve_claim before cooldown/
    // vesting starts.
    s.client.approve_claim(&claim_id);
    advance_days(&env, 7);
    advance_days(&env, 10);

    env.cost_estimate().budget().reset_default();
    s.client.claim_stream(&claim_id, &ben);
    print_budget(&env, "claim_stream (partial vesting, hits the daily-outflow-cap path)");

    assert!(env.cost_estimate().budget().cpu_instruction_cost() < MAX_CPU_INSTRUCTIONS);
}

#[test]
fn profile_approve_override() {
    // The 2-of-2 flow's execution branch, most logic-dense single call
    // in the contract (forfeiture + solvency check + claim creation in
    // one invocation).
    let env = new_env();
    let s = setup(&env);
    let (staker, _ben) = staked_wallet(&env, &s);
    let hash = tx_hash(&env, 1);
    s.client
        .approve_override(&s.admin, &staker, &hash, &STROOPS_PER_UNIT, &TIER_C);

    env.cost_estimate().budget().reset_default();
    s.client
        .approve_override(&s.co_signer, &staker, &hash, &STROOPS_PER_UNIT, &TIER_C);
    print_budget(&env, "approve_override (second approval, executes)");

    assert!(env.cost_estimate().budget().cpu_instruction_cost() < MAX_CPU_INSTRUCTIONS);
}
