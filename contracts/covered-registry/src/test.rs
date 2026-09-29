#![cfg(test)]
//! One test per locked rule: permanent one-staker binding, max 3, writer-only
//! writes, coverage = live stake in the REAL pool (and back on re-stake),
//! events, TTL extension.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, BytesN, Env};

use crate::{CoveredRegistry, CoveredRegistryClient, RegistryError, MAX_WALLETS_PER_STAKER};
use protection_pool::{GovChange, GovKind, ProtectionPool, ProtectionPoolClient};

/// The pool's governance wait, mirrored from the contract.
const GOV_DELAY_SECONDS: u64 = 7 * 86_400;

const POOL_CAP: i128 = 100_000_000_000;
const STAKE: i128 = 55_000_000; // inside the pool's 0.01%-0.1% of cap bounds

struct T<'a> {
    reg: CoveredRegistryClient<'a>,
    pool: ProtectionPoolClient<'a>,
    token: StellarAssetClient<'a>,
    admin: Address,
    co_signer: Address,
    writer: Address,
}

fn setup(env: &Env) -> T<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
    });
    let admin = Address::generate(env);
    let co_signer = Address::generate(env);
    let writer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = StellarAssetClient::new(env, &sac.address());
    let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let pool_id = env.register(
        ProtectionPool,
        (
            admin.clone(),
            Address::generate(env),
            BytesN::from_array(env, &key.verifying_key().to_bytes()),
            co_signer.clone(),
            Address::generate(env), // guardian
            sac.address(),
            POOL_CAP,
        ),
    );
    let reg_id = env.register(CoveredRegistry, (writer.clone(), pool_id.clone()));
    T {
        reg: CoveredRegistryClient::new(env, &reg_id),
        pool: ProtectionPoolClient::new(env, &pool_id),
        token,
        admin,
        co_signer,
        writer,
    }
}

fn wallet(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

fn stake(env: &Env, t: &T<'_>, staker: &Address) {
    t.token.mint(staker, &STAKE);
    t.pool.stake(staker, &STAKE, &Address::generate(env));
}

fn emitted(env: &Env, name: &str) -> bool {
    env.events().all().events().iter().any(|e| match &e.body {
        ContractEventBody::V0(v0) => v0
            .topics
            .first()
            .map_or(false, |t| matches!(t, ScVal::Symbol(s) if s.0.as_slice() == name.as_bytes())),
    })
}

#[test]
fn register_records_staker_and_time() {
    let env = Env::default();
    let t = setup(&env);
    let staker = Address::generate(&env);
    t.reg.register(&wallet(&env, 1), &staker);
    assert!(emitted(&env, "wallet_registered"));

    let r = t.reg.get_registration(&wallet(&env, 1)).unwrap();
    assert_eq!(r.staker, staker);
    assert_eq!(r.registered_at, env.ledger().timestamp());
    assert_eq!(t.reg.get_wallets(&staker).len(), 1);
}

#[test]
fn only_the_writer_signs_a_registration() {
    let env = Env::default();
    let t = setup(&env);
    let staker = Address::generate(&env);
    t.reg.register(&wallet(&env, 1), &staker);
    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, t.writer);
}

#[test]
fn unauthorised_caller_cannot_register() {
    let env = Env::default();
    let t = setup(&env);
    env.set_auths(&[]); // nobody signs
    assert!(t.reg.try_register(&wallet(&env, 1), &Address::generate(&env)).is_err());
    assert!(t.reg.get_registration(&wallet(&env, 1)).is_none());
}

#[test]
fn one_wallet_one_staker_forever() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    t.reg.register(&wallet(&env, 1), &a);
    assert_eq!(
        t.reg.try_register(&wallet(&env, 1), &b),
        Err(Ok(RegistryError::WalletTakenByOtherStaker))
    );
    assert_eq!(
        t.reg.try_register(&wallet(&env, 1), &a),
        Err(Ok(RegistryError::AlreadyRegistered))
    );
    assert_eq!(t.reg.get_registration(&wallet(&env, 1)).unwrap().staker, a);
}

#[test]
fn max_three_wallets_per_staker() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    for i in 0..MAX_WALLETS_PER_STAKER as u8 {
        t.reg.register(&wallet(&env, i + 1), &a);
    }
    assert_eq!(
        t.reg.try_register(&wallet(&env, 9), &a),
        Err(Ok(RegistryError::StakerLimitReached))
    );
    assert_eq!(t.reg.get_wallets(&a).len(), MAX_WALLETS_PER_STAKER);
    assert!(t.reg.get_registration(&wallet(&env, 9)).is_none());
}

#[test]
fn limit_is_per_staker() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    for i in 0..3u8 {
        t.reg.register(&wallet(&env, i + 1), &a);
    }
    t.reg.register(&wallet(&env, 10), &b);
    assert_eq!(t.reg.get_wallets(&b).len(), 1);
}

#[test]
fn coverage_follows_the_live_stake_and_returns_on_restake() {
    let env = Env::default();
    let t = setup(&env);
    let staker = Address::generate(&env);
    let ben = Address::generate(&env);
    t.reg.register(&wallet(&env, 1), &staker);
    assert!(!t.reg.is_covered(&wallet(&env, 1)), "registered but not staking");

    t.token.mint(&staker, &STAKE);
    t.pool.stake(&staker, &STAKE, &ben);
    assert!(t.reg.is_covered(&wallet(&env, 1)), "staking = covered");

    t.pool.withdraw(&staker, &ben);
    assert!(!t.reg.is_covered(&wallet(&env, 1)), "withdrawn = not covered");
    assert_eq!(t.reg.get_registration(&wallet(&env, 1)).unwrap().staker, staker, "still bound");

    stake(&env, &t, &staker);
    assert!(t.reg.is_covered(&wallet(&env, 1)), "same wallet covered again on re-stake");
}

#[test]
fn suspended_stake_is_not_covered() {
    let env = Env::default();
    let t = setup(&env);
    let staker = Address::generate(&env);
    t.reg.register(&wallet(&env, 1), &staker);
    stake(&env, &t, &staker);
    t.pool.suspend_stake(&staker);
    assert!(!t.reg.is_covered(&wallet(&env, 1)));
}

#[test]
fn unknown_wallet_is_not_covered() {
    let env = Env::default();
    let t = setup(&env);
    assert!(!t.reg.is_covered(&wallet(&env, 1)));
}

#[test]
fn writer_changes_only_through_pool_governance() {
    let env = Env::default();
    let t = setup(&env);
    let new_writer = Address::generate(&env);
    let change = GovChange::RegistryWriter(t.reg.address.clone(), new_writer.clone());

    t.pool.propose_change(&t.admin, &change);
    t.pool.approve_change(&t.co_signer, &change);
    assert!(t.pool.try_execute_change(&GovKind::RegistryWriter).is_err());
    assert_eq!(t.reg.get_writer(), t.writer);

    env.ledger().with_mut(|li| li.timestamp += GOV_DELAY_SECONDS);
    t.pool.execute_change(&GovKind::RegistryWriter);
    assert!(emitted(&env, "writer_changed"));
    assert_eq!(t.reg.get_writer(), new_writer);

    t.reg.register(&wallet(&env, 1), &Address::generate(&env));
    assert_eq!(env.auths()[0].0, new_writer);
}

#[test]
fn nobody_but_the_pool_can_set_the_writer() {
    let env = Env::default();
    let t = setup(&env);
    // Without mocked signatures, a direct call cannot present the pool's auth.
    env.set_auths(&[]);
    assert!(t.reg.try_set_writer(&Address::generate(&env)).is_err());
    assert_eq!(t.reg.get_writer(), t.writer);
}

#[test]
fn extend_ttl_is_permissionless_and_needs_a_registration() {
    let env = Env::default();
    let t = setup(&env);
    assert_eq!(t.reg.try_extend_ttl(&wallet(&env, 1)), Err(Ok(RegistryError::NotRegistered)));
    t.reg.register(&wallet(&env, 1), &Address::generate(&env));
    t.reg.extend_ttl(&wallet(&env, 1));
    assert!(env.auths().is_empty());
}

#[test]
fn pool_address_is_fixed() {
    let env = Env::default();
    let t = setup(&env);
    assert_eq!(t.reg.get_pool(), t.pool.address);
}
