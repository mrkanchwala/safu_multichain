//! Fuzz target for the 2-of-2 override/rotation flow, the module where
//! 6 real bugs were found by hand during this session's audit (stale
//! approval surviving a coSigner rotation, double-release of
//! total_allocated on re-execution, false-insolvent on same-claim
//! correction, etc., found in the 2026-07-14 security review). fuzz_solvency.rs never
//! calls approve_override/cancel_pending_override/transfer_admin/
//! set_co_signer at all, so this is genuinely new coverage, not a
//! duplicate of the first target.
//!
//! Two candidate admins and two candidate coSigners exist so rotation
//! mid-override-approval is reachable: a call can name either the
//! current or the "other" identity, and TransferAdmin/SetCoSigner can
//! fire between two ApproveOverride calls on the same (wallet, tx_hash)
//! pair: exactly the sequence that produced the stale-approval bug.
//!
//! Run: `cargo +nightly fuzz run fuzz_override -- -max_total_time=120`

#![no_main]

//! T2/D1 note: same reasoning as fuzz_solvency.rs, `submit_claim`'s oracle
//! path now verifies an Ed25519 signature, so this target signs each
//! generated claim correctly instead of fuzzing the signature bytes, which
//! would trap on essentially every input and be reported as a crash.

use arbitrary::Arbitrary;
use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env};

use protection_pool::{GovChange, GovKind, ProtectionPool, ProtectionPoolClient};

/// Governance public wait (7 days), mirrored from the contract.
const GOV_DELAY: u64 = 7 * 86_400;

const POOL_CAP: i128 = 100_000_000_000;
const NUM_WALLETS: usize = 3;
const NUM_TX_HASHES: usize = 3;

#[derive(Arbitrary, Debug)]
enum Op {
    Stake { wallet: u8, amount: i128 },
    SubmitClaim { wallet: u8, tx: u8, entitlement: i128, tier: u8 },
    ApproveOverride { caller_is_admin_slot: bool, actor: u8, wallet: u8, tx: u8, entitlement: i128, tier: u8 },
    CancelPendingOverride { wallet: u8, tx: u8 },
    TransferAdmin { new_admin: u8 },
    SetCoSigner { new_co_signer: u8 },
    ClaimStream { claim_idx: u8 },
    CancelClaim { claim_idx: u8 },
    AdvanceDays { days: u8 },
}

fuzz_target!(|ops: Vec<Op>| {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
    });

    // Two candidate identities per role, lets ops pick "current" or
    // "other" so rotation is exercised for real.
    let admins: Vec<Address> = (0..2).map(|_| Address::generate(&env)).collect();
    let co_signers: Vec<Address> = (0..2).map(|_| Address::generate(&env)).collect();
    let oracle = Address::generate(&env);
    // Pre-audit hardening: third governance role; roles rotate only through
    // governance (two roles + 7 days), exercised below.
    let guardian = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(admins[0].clone());
    let token_id = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token_id);

    let oracle_key = SigningKey::from_bytes(&[7u8; 32]);
    let oracle_pubkey = BytesN::from_array(&env, &oracle_key.verifying_key().to_bytes());

    // Configured via `__constructor` at registration (7a audit, Finding 3,
    // the separate `initialize` entrypoint was removed to close the
    // deploy->init front-running window).
    let contract_id = env.register(
        ProtectionPool,
        (
            admins[0].clone(),
            oracle.clone(),
            oracle_pubkey.clone(),
            co_signers[0].clone(),
            guardian.clone(),
            token_id.clone(),
            POOL_CAP,
        ),
    );
    let client = ProtectionPoolClient::new(&env, &contract_id);

    let wallets: Vec<Address> = (0..NUM_WALLETS)
        .map(|_| {
            let a = Address::generate(&env);
            token_admin.mint(&a, &POOL_CAP);
            a
        })
        .collect();
    let beneficiaries: Vec<Address> = (0..NUM_WALLETS).map(|_| Address::generate(&env)).collect();
    let tx_hashes: Vec<BytesN<32>> = (0..NUM_TX_HASHES)
        .map(|i| BytesN::from_array(&env, &[i as u8; 32]))
        .collect();

    let mut claims: Vec<(BytesN<32>, Address)> = Vec::new();

    // No `get_admin`/`get_co_signer` view exists on the contract (checked
    // the actual exported view list in lib.rs), tracked locally instead,
    // updated only when a rotation call actually succeeds. This is a
    // harness-only concern, not a reason to add a new contract view.
    let mut current_admin = admins[0].clone();

    for op in ops {
        match op {
            Op::Stake { wallet, amount } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let _ = client.try_stake(&wallets[i], &amount, &beneficiaries[i]);
            }
            Op::SubmitClaim { wallet, tx, entitlement, tier } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let h = &tx_hashes[(tx as usize) % NUM_TX_HASHES];
                let now = env.ledger().timestamp();
                let tier_val = 1 + (tier % 3) as u32;
                let deadline = now + 3_600;
                let payload = env.as_contract(&contract_id, || {
                    protection_pool::testutils::build_approval_payload(
                        &env,
                        &wallets[i],
                        h,
                        entitlement,
                        tier_val,
                        now,
                        deadline,
                    )
                });
                let msg: Vec<u8> = payload.iter().collect();
                let sig = BytesN::from_array(&env, &oracle_key.sign(&msg).to_bytes());
                if let Ok(Ok(id)) = client.try_submit_claim(
                    &oracle, &wallets[i], h, &entitlement, &tier_val, &now, &deadline, &sig,
                ) {
                    claims.push((id, beneficiaries[i].clone()));
                }
            }
            Op::ApproveOverride { caller_is_admin_slot, actor, wallet, tx, entitlement, tier } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let h = &tx_hashes[(tx as usize) % NUM_TX_HASHES];
                let tier_val = 1 + (tier % 3) as u32;
                let caller = if caller_is_admin_slot {
                    &admins[(actor as usize) % admins.len()]
                } else {
                    &co_signers[(actor as usize) % co_signers.len()]
                };
                let _ = client.try_approve_override(
                    caller, &wallets[i], h, &entitlement, &tier_val,
                );
            }
            Op::CancelPendingOverride { wallet, tx } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let h = &tx_hashes[(tx as usize) % NUM_TX_HASHES];
                // Only the CURRENT admin can cancel, matches V8's onlyOwner.
                let _ = client.try_cancel_pending_override(&current_admin, &wallets[i], h);
            }
            Op::TransferAdmin { new_admin } => {
                let new = admins[(new_admin as usize) % admins.len()].clone();
                let change = GovChange::Admin(new.clone());
                let co = client.get_co_signer();
                if client.try_propose_change(&co, &change).is_ok() {
                    let _ = client.try_approve_change(&guardian, &change);
                    env.ledger().with_mut(|li| {
                        li.timestamp += GOV_DELAY;
                        li.sequence_number += (GOV_DELAY / 5) as u32;
                    });
                    if client.try_execute_change(&GovKind::Admin).is_ok() {
                        current_admin = new;
                    } else {
                        let _ = client.try_cancel_change(&co, &GovKind::Admin);
                        let _ = client.try_cancel_change(&guardian, &GovKind::Admin);
                    }
                }
                assert_eq!(client.get_admin(), current_admin, "admin changed outside governance");
            }
            Op::SetCoSigner { new_co_signer } => {
                let new = co_signers[(new_co_signer as usize) % co_signers.len()].clone();
                let change = GovChange::CoSigner(new);
                if client.try_propose_change(&current_admin, &change).is_ok() {
                    let _ = client.try_approve_change(&guardian, &change);
                    env.ledger().with_mut(|li| {
                        li.timestamp += GOV_DELAY;
                        li.sequence_number += (GOV_DELAY / 5) as u32;
                    });
                    if client.try_execute_change(&GovKind::CoSigner).is_err() {
                        let _ = client.try_cancel_change(&current_admin, &GovKind::CoSigner);
                        let _ = client.try_cancel_change(&guardian, &GovKind::CoSigner);
                    }
                }
            }
            Op::ClaimStream { claim_idx } => {
                if !claims.is_empty() {
                    let (id, ben) = &claims[(claim_idx as usize) % claims.len()];
                    let _ = client.try_claim_stream(id, ben);
                }
            }
            Op::CancelClaim { claim_idx } => {
                if !claims.is_empty() {
                    let (id, _) = &claims[(claim_idx as usize) % claims.len()];
                    let _ = client.try_cancel_claim(id);
                }
            }
            Op::AdvanceDays { days } => {
                let d = 1u32 + (days as u32 % 100);
                env.ledger().with_mut(|li| {
                    li.sequence_number += d * 17_280;
                    li.timestamp += (d as u64) * 86_400;
                });
            }
        }

        let total_allocated = client.get_total_allocated();
        let total_staked = client.get_total_staked();
        assert!(
            total_allocated <= total_staked,
            "SOLVENCY INVARIANT VIOLATED: total_allocated={} > total_staked={}",
            total_allocated,
            total_staked
        );
        assert!(total_allocated >= 0, "total_allocated went negative: {}", total_allocated);
        assert!(total_staked >= 0, "total_staked went negative: {}", total_staked);
    }
});
