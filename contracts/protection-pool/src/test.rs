//! Unit test suite for ProtectionPool (SCF #44 Tranche 1 D2 deliverable:
//! ≥100 tests). Split by mechanic rather than one file, see each
//! submodule for what it covers. `common.rs` holds shared setup, not
//! tests itself.

#![cfg(test)]

mod admin_tests;
/// v1 (2026-09-22): backers, one test per locked withdrawal-safety rule.
mod backer_tests;
mod blend_scenario_tests;
mod claim_tests;
mod common;
/// T2/D1: on-chain Ed25519 oracle approval verification. Split out rather
/// than folded into `claim_tests` because it tests the signature gate
/// itself, not claim mechanics.
mod d1_signature_tests;
/// T2/D2: DeFindex vault yield deployment. Split out for the same reason
/// as `d1_signature_tests`: this covers the liquid-vs-deployed accounting
/// layer and its invariant, not pool mechanics.
mod d2_vault_tests;
/// Pre-audit gate P2 (2026-09-23): key-loss-safe governance scenarios.
mod governance_tests;
/// v1 (2026-09-24): in-path two-way rebalancing, plus adversarial stress on
/// every exit path (can the pull/push be gamed?).
mod inpath_liquidity_tests;
mod mutation_gap_tests;
mod override_tests;
/// Pre-audit gate P2 (2026-09-23): the two `/audit-chain` findings.
mod p2_findings_tests;
mod pool_demo_tests;
mod pool_solvency_tests;
mod profiling_tests;
mod settings_tests;
mod solvency_tests;
mod stake_tests;
mod t2_mutation_gap_tests;
/// T3 (2026-08-24), admission-side retry queue, bidirectional liquidity
/// rebalancing (`ensure_liquidity`/`auto_deploy_liquidity`), and the
/// `total_staked` shortfall reconciliation. Split out for the same reason
/// as `d2_vault_tests`: new mechanic, not existing pool mechanics.
mod t3_flags_tests;
