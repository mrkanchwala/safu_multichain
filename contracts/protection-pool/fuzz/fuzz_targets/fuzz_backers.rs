//! v1 (2026-09-22): fuzz target for backers, run against the full claim
//! state machine (stakers, claims, streams, cancels, pause, time).
//!
//! Invariants, checked after EVERY op:
//!   1. Conservation per backer: tokens in the backer's wallet + what the
//!      pool records for them == what they were funded with. Nothing can move
//!      backer money anywhere except back to the backer (rule 1), and nothing
//!      is ever deducted (full-amount return).
//!   2. Totals match the sum of records: total_backed == sum(amount),
//!      total_backed_pending == sum(pending_amount). All non-negative.
//!   3. An open request never exceeds the matured balance.
//!   4. Rule 3: every SUCCESSFUL backer withdrawal leaves
//!      total_allocated <= capacity.
//!
//! Run: `cargo +nightly fuzz run fuzz_backers -- -max_total_time=120`

#![no_main]

use arbitrary::Arbitrary;
use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, BytesN, Env};

use protection_pool::{ProtectionPool, ProtectionPoolClient};

const POOL_CAP: i128 = 100_000_000_000;
const NUM_WALLETS: usize = 4;
const NUM_BACKERS: usize = 3;
const BACKER_FUNDING: i128 = 10_000_000_000;

#[derive(Arbitrary, Debug)]
enum Op {
    Stake { wallet: u8, amount: i128 },
    Withdraw { wallet: u8 },
    SubmitClaim { wallet: u8, entitlement: i128, tier: u8 },
    ApproveClaim { claim_idx: u8 },
    ClaimStream { claim_idx: u8 },
    CancelClaim { claim_idx: u8 },
    Back { backer: u8, amount: i128 },
    Mature { backer: u8 },
    Request { backer: u8, amount: i128 },
    Cancel { backer: u8 },
    Complete { backer: u8 },
    Pause,
    Unpause,
    AdvanceDays { days: u8 },
}

fuzz_target!(|ops: Vec<Op>| {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
    });

    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let co_signer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token_id);
    let token = TokenClient::new(&env, &token_id);

    let oracle_key = SigningKey::from_bytes(&[7u8; 32]);
    let oracle_pubkey = BytesN::from_array(&env, &oracle_key.verifying_key().to_bytes());
    let contract_id = env.register(
        ProtectionPool,
        (admin.clone(), oracle.clone(), oracle_pubkey, co_signer.clone(), Address::generate(&env), token_id.clone(), POOL_CAP),
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
    let backers: Vec<Address> = (0..NUM_BACKERS)
        .map(|_| {
            let a = Address::generate(&env);
            token_admin.mint(&a, &BACKER_FUNDING);
            a
        })
        .collect();

    let mut claims: Vec<(BytesN<32>, Address)> = Vec::new();

    for op in ops {
        match op {
            Op::Stake { wallet, amount } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let _ = client.try_stake(&wallets[i], &amount, &beneficiaries[i]);
            }
            Op::Withdraw { wallet } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let _ = client.try_withdraw(&wallets[i], &beneficiaries[i]);
            }
            Op::SubmitClaim { wallet, entitlement, tier } => {
                let i = (wallet as usize) % NUM_WALLETS;
                let hash = BytesN::from_array(&env, &[claims.len() as u8; 32]);
                let now = env.ledger().timestamp();
                let tier_val = 1 + (tier % 3) as u32;
                let deadline = now + 3_600;
                let payload = env.as_contract(&contract_id, || {
                    protection_pool::testutils::build_approval_payload(
                        &env, &wallets[i], &hash, entitlement, tier_val, now, deadline,
                    )
                });
                let msg: Vec<u8> = payload.iter().collect();
                let sig = BytesN::from_array(&env, &oracle_key.sign(&msg).to_bytes());
                if let Ok(Ok(id)) = client.try_submit_claim(
                    &oracle, &wallets[i], &hash, &entitlement, &tier_val, &now, &deadline, &sig,
                ) {
                    claims.push((id, beneficiaries[i].clone()));
                }
            }
            Op::ApproveClaim { claim_idx } => {
                if !claims.is_empty() {
                    let (id, _) = &claims[(claim_idx as usize) % claims.len()];
                    let _ = client.try_unlock_pending_claim(id);
                    let _ = client.try_try_release_queued_claim(id);
                    let _ = client.try_approve_claim(id);
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
            Op::Back { backer, amount } => {
                let i = (backer as usize) % NUM_BACKERS;
                let _ = client.try_back(&backers[i], &amount);
            }
            Op::Mature { backer } => {
                let _ = client.try_mature_backing(&backers[(backer as usize) % NUM_BACKERS]);
            }
            Op::Request { backer, amount } => {
                let i = (backer as usize) % NUM_BACKERS;
                let _ = client.try_request_backer_withdrawal(&backers[i], &amount);
            }
            Op::Cancel { backer } => {
                let _ = client.try_cancel_backer_withdrawal(&backers[(backer as usize) % NUM_BACKERS]);
            }
            Op::Complete { backer } => {
                let i = (backer as usize) % NUM_BACKERS;
                if let Ok(Ok(_)) = client.try_complete_backer_withdrawal(&backers[i]) {
                    let allocated = client.get_total_allocated();
                    let capacity = client.get_capacity();
                    assert!(
                        allocated <= capacity,
                        "RULE 3 VIOLATED: withdrawal left allocated={} > capacity={}",
                        allocated,
                        capacity
                    );
                }
            }
            Op::Pause => client.pause(),
            Op::Unpause => client.unpause(),
            Op::AdvanceDays { days } => {
                let d = 1u32 + (days as u32 % 100);
                env.ledger().with_mut(|li| {
                    li.sequence_number += d * 17_280;
                    li.timestamp += (d as u64) * 86_400;
                });
            }
        }

        let mut sum_amount = 0i128;
        let mut sum_pending = 0i128;
        for b in &backers {
            let (amount, pending, requested) = match client.get_backer(b) {
                Some(r) => (r.amount, r.pending_amount, r.withdraw_amount),
                None => (0, 0, 0),
            };
            assert!(amount >= 0 && pending >= 0 && requested >= 0, "negative backer field");
            assert!(requested <= amount, "request {} exceeds matured {}", requested, amount);
            assert_eq!(
                token.balance(b) + amount + pending,
                BACKER_FUNDING,
                "BACKER CONSERVATION VIOLATED: money moved or was deducted"
            );
            sum_amount += amount;
            sum_pending += pending;
        }
        assert_eq!(client.get_total_backed(), sum_amount, "total_backed drift");
        assert_eq!(client.get_total_backed_pending(), sum_pending, "total_backed_pending drift");
    }
});
