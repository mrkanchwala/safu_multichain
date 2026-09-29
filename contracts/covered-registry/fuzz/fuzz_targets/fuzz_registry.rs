//! v1 (2026-09-23, pre-audit hardening): fuzz target for the covered-wallet
//! registry, run against the REAL pool.
//!
//! A shadow model tracks what the locked rules say must be true; every op's
//! result is compared to the model, and the whole registry is re-read after
//! EVERY op.
//!
//! Invariants:
//!   1. `register` returns exactly what the rules predict: Ok, AlreadyRegistered,
//!      WalletTakenByOtherStaker or StakerLimitReached. Nothing else.
//!   2. A registration never changes once made: same staker, same
//!      `registered_at`, forever (no deregister, swap or override).
//!   3. A staker never has more than MAX_WALLETS_PER_STAKER wallets, and
//!      `get_wallets` lists exactly the model's wallets, in order.
//!   4. Only the CURRENT writer's signature registers; the writer changes
//!      only through the pool's governance (two roles + 7 days), never by a
//!      direct call. An unsigned call changes nothing.
//!   5. `is_covered` == registered AND the staker is eligible in the pool.
//!   6. `extend_ttl` is permissionless and fails only for an unknown wallet.
//!
//! Run: `cargo +nightly fuzz run fuzz_registry -- -max_total_time=600`

#![no_main]

use std::collections::HashMap;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env};

use covered_registry::{CoveredRegistry, CoveredRegistryClient, RegistryError, MAX_WALLETS_PER_STAKER};
use protection_pool::{GovChange, GovKind, ProtectionPool, ProtectionPoolClient};

/// The pool's governance wait (7 days), mirrored from the contract.
const GOV_DELAY_DAYS: u32 = 7;

const POOL_CAP: i128 = 100_000_000_000;
const STAKE: i128 = 55_000_000; // inside the pool's stake bounds for this cap
const NUM_STAKERS: usize = 4;
const NUM_WALLETS: u8 = 16;
const MAX_OPS: usize = 64;
/// Keeps total time travel under the entry TTL set below, so archival never
/// masquerades as a registry bug.
const MAX_TOTAL_DAYS: u32 = 365;

#[derive(Arbitrary, Debug)]
enum Op {
    Register { wallet: u8, staker: u8 },
    RegisterUnsigned { wallet: u8, staker: u8 },
    RotateWriter,
    RotateWriterUnsigned,
    Stake { staker: u8 },
    Withdraw { staker: u8 },
    Suspend { staker: u8 },
    ExtendTtl { wallet: u8 },
    AdvanceDays { days: u8 },
}

fuzz_target!(|ops: Vec<Op>| {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
        li.min_persistent_entry_ttl = 10_000_000;
        li.min_temp_entry_ttl = 10_000_000;
        li.max_entry_ttl = 20_000_000;
    });

    let admin = Address::generate(&env);
    let co_signer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = StellarAssetClient::new(&env, &sac.address());
    let oracle_pubkey = BytesN::from_array(&env, &[9u8; 32]); // no claims here
    let pool_id = env.register(
        ProtectionPool,
        (admin.clone(), Address::generate(&env), oracle_pubkey, co_signer.clone(), Address::generate(&env), sac.address(), POOL_CAP),
    );
    let pool = ProtectionPoolClient::new(&env, &pool_id);
    let mut writer = Address::generate(&env);
    let reg_id = env.register(CoveredRegistry, (writer.clone(), pool_id.clone()));
    let reg = CoveredRegistryClient::new(&env, &reg_id);

    let stakers: Vec<Address> = (0..NUM_STAKERS).map(|_| Address::generate(&env)).collect();
    let hash = |w: u8| BytesN::from_array(&env, &[w; 32]);

    // Shadow model: wallet -> (staker index, registered_at); staker -> wallets.
    let mut model: HashMap<u8, (usize, u64)> = HashMap::new();
    let mut lists: Vec<Vec<u8>> = vec![Vec::new(); NUM_STAKERS];
    let mut days_used = 0u32;

    for op in ops.into_iter().take(MAX_OPS) {
        match op {
            Op::Register { wallet, staker } => {
                let w = wallet % NUM_WALLETS;
                let s = (staker as usize) % NUM_STAKERS;
                let expected = match model.get(&w) {
                    Some((owner, _)) if *owner == s => Err(RegistryError::AlreadyRegistered),
                    Some(_) => Err(RegistryError::WalletTakenByOtherStaker),
                    None if lists[s].len() as u32 >= MAX_WALLETS_PER_STAKER => {
                        Err(RegistryError::StakerLimitReached)
                    }
                    None => Ok(()),
                };
                let got = reg.try_register(&hash(w), &stakers[s]);
                match expected {
                    Ok(()) => {
                        assert!(matches!(got, Ok(Ok(()))), "register w{w} s{s}: expected Ok, got {got:?}");
                        let auths = env.auths();
                        assert_eq!(auths.len(), 1, "exactly one signer");
                        assert_eq!(auths[0].0, writer, "signed by someone other than the current writer");
                        model.insert(w, (s, env.ledger().timestamp()));
                        lists[s].push(w);
                    }
                    Err(e) => assert_eq!(got, Err(Ok(e)), "register w{w} s{s}"),
                }
            }
            Op::RegisterUnsigned { wallet, staker } => {
                let w = wallet % NUM_WALLETS;
                let s = (staker as usize) % NUM_STAKERS;
                env.set_auths(&[]);
                assert!(reg.try_register(&hash(w), &stakers[s]).is_err(), "unsigned register succeeded");
                env.mock_all_auths();
            }
            Op::RotateWriter => {
                if days_used + GOV_DELAY_DAYS <= MAX_TOTAL_DAYS {
                    let new_writer = Address::generate(&env);
                    let change = GovChange::RegistryWriter(reg_id.clone(), new_writer.clone());
                    pool.propose_change(&admin, &change);
                    pool.approve_change(&co_signer, &change);
                    assert!(pool.try_execute_change(&GovKind::RegistryWriter).is_err(), "writer changed before the delay");
                    assert_eq!(reg.get_writer(), writer);
                    days_used += GOV_DELAY_DAYS;
                    env.ledger().with_mut(|li| {
                        li.sequence_number += GOV_DELAY_DAYS * 17_280;
                        li.timestamp += GOV_DELAY_DAYS as u64 * 86_400;
                    });
                    pool.execute_change(&GovKind::RegistryWriter);
                    writer = new_writer;
                }
            }
            Op::RotateWriterUnsigned => {
                // Nobody but the pool may set the writer: with no mocked
                // signatures a direct call cannot present the pool's auth.
                env.set_auths(&[]);
                assert!(reg.try_set_writer(&Address::generate(&env)).is_err(), "set_writer succeeded outside the pool");
                env.mock_all_auths();
            }
            Op::Stake { staker } => {
                let st = &stakers[(staker as usize) % NUM_STAKERS];
                token.mint(st, &STAKE);
                let _ = pool.try_stake(st, &STAKE, st);
            }
            Op::Withdraw { staker } => {
                let st = &stakers[(staker as usize) % NUM_STAKERS];
                let _ = pool.try_withdraw(st, st);
            }
            Op::Suspend { staker } => {
                let _ = pool.try_suspend_stake(&stakers[(staker as usize) % NUM_STAKERS]);
            }
            Op::ExtendTtl { wallet } => {
                let w = wallet % NUM_WALLETS;
                let got = reg.try_extend_ttl(&hash(w));
                if model.contains_key(&w) {
                    assert!(matches!(got, Ok(Ok(()))), "extend_ttl on a registered wallet failed: {got:?}");
                    assert!(env.auths().is_empty(), "extend_ttl demanded a signature");
                } else {
                    assert_eq!(got, Err(Ok(RegistryError::NotRegistered)));
                }
            }
            Op::AdvanceDays { days } => {
                let d = 1 + (days as u32 % 30);
                if days_used + d <= MAX_TOTAL_DAYS {
                    days_used += d;
                    env.ledger().with_mut(|li| {
                        li.sequence_number += d * 17_280;
                        li.timestamp += d as u64 * 86_400;
                    });
                }
            }
        }

        // Full re-read against the model after every op.
        for w in 0..NUM_WALLETS {
            let on_chain = reg.get_registration(&hash(w));
            match model.get(&w) {
                Some((s, at)) => {
                    let r = on_chain.expect("registration vanished");
                    assert_eq!(r.staker, stakers[*s], "registration moved to another staker");
                    assert_eq!(r.registered_at, *at, "registered_at changed");
                    let eligible = pool.is_eligible(&stakers[*s]);
                    assert_eq!(reg.is_covered(&hash(w)), eligible, "is_covered != staker eligibility");
                }
                None => {
                    assert!(on_chain.is_none(), "registration appeared outside register()");
                    assert!(!reg.is_covered(&hash(w)), "unregistered wallet covered");
                }
            }
        }
        for (s, st) in stakers.iter().enumerate() {
            let listed = reg.get_wallets(st);
            assert!(listed.len() <= MAX_WALLETS_PER_STAKER, "staker over the wallet limit");
            let listed: Vec<BytesN<32>> = listed.iter().collect();
            let expected: Vec<BytesN<32>> = lists[s].iter().map(|w| hash(*w)).collect();
            assert_eq!(listed, expected, "get_wallets drifted from the model");
        }
        assert_eq!(reg.get_writer(), writer, "writer drifted");
        assert_eq!(reg.get_pool(), pool_id, "pool address changed");
    }
});
