// SPDX-License-Identifier: Apache-2.0

//! SAFU covered-wallet registry (v1, 2026-09-22).
//!
//! Records which staker covers which wallet. A separate contract, never part
//! of `protection-pool` (mechanism review 2026-09-22: in the pool it gives
//! neither uniqueness nor enforcement).
//!
//! Rules (founder, locked 2026-09-22):
//! - Up to `MAX_WALLETS_PER_STAKER` (3) wallets per staker. A constant.
//! - A wallet belongs to ONE staker, and FOREVER. There is no deregister,
//!   swap or admin override: nothing in this contract removes or moves a
//!   registration.
//! - Coverage = active stake. A registered wallet is covered only while its
//!   staker holds a live stake in the pool (`is_covered`). If the staker
//!   stakes again later, the same wallets are covered again.
//! - One claim per stake, up to 20 hacks, capped at the tier ceiling: enforced
//!   by the pool and the backend, not here.
//!
//! Key: `wallet_hash = sha256(chain_id || normalized_wallet)`, computed by the
//! backend (wallets live on other chains; there is nothing for this contract
//! to parse). No private keys are ever involved. A public address hash can be
//! recomputed by anyone, so this registry does NOT hide who covers a wallet.
//!
//! Writer: one address (the backend key) registers wallets after an off-chain
//! ownership proof (same-chain link, or a send-to-yourself; 2026-09-23).
//! Because registrations are permanent, a compromised writer could bind a
//! wallet to the wrong staker for good: the writer key belongs on KMS.
//!
//! Writer changes (pre-audit gate P2, 2026-09-23): only the pool can set a
//! new writer, through the pool's own governance (any 2 of its 3 roles, 7
//! days public, with 90-day recovery if keys are lost). One governance for
//! the whole system, so the registry can never be stranded by a lost key.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, vec, Address, BytesN, Env,
    IntoVal, Symbol, Val, Vec,
};

pub const MAX_WALLETS_PER_STAKER: u32 = 3;

const LEDGERS_PER_DAY: u32 = 17_280;
const BUMP_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
const BUMP_TO: u32 = 120 * LEDGERS_PER_DAY;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RegistryError {
    /// This staker already registered this wallet.
    AlreadyRegistered = 1,
    /// Another staker registered this wallet. Registrations are permanent.
    WalletTakenByOtherStaker = 2,
    /// The staker already has `MAX_WALLETS_PER_STAKER` wallets.
    StakerLimitReached = 3,
    /// Nothing registered under this wallet hash.
    NotRegistered = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub staker: Address,
    /// Ledger timestamp of registration. The backend's claim rules use it
    /// (registered at least 7 days before the hack).
    pub registered_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Writer,
    Pool,
    Wallet(BytesN<32>),
    StakerWallets(Address),
}

#[contractevent]
pub struct WalletRegistered {
    #[topic]
    pub staker: Address,
    #[topic]
    pub wallet_hash: BytesN<32>,
    pub registered_at: u64,
}

#[contractevent]
pub struct WriterChanged {
    pub old_writer: Address,
    pub new_writer: Address,
}

#[contract]
pub struct CoveredRegistry;

fn bump_instance(env: &Env) {
    env.storage().instance().extend_ttl(BUMP_THRESHOLD, BUMP_TO);
}

fn bump_wallet(env: &Env, wallet_hash: &BytesN<32>) {
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::Wallet(wallet_hash.clone()), BUMP_THRESHOLD, BUMP_TO);
}

fn bump_staker(env: &Env, staker: &Address) {
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::StakerWallets(staker.clone()), BUMP_THRESHOLD, BUMP_TO);
}

#[contractimpl]
impl CoveredRegistry {
    /// `pool` is fixed for the life of this registry.
    pub fn __constructor(env: Env, writer: Address, pool: Address) {
        env.storage().instance().set(&DataKey::Writer, &writer);
        env.storage().instance().set(&DataKey::Pool, &pool);
        bump_instance(&env);
    }

    /// Writer only. Binds `wallet_hash` to `staker` permanently.
    pub fn register(env: Env, wallet_hash: BytesN<32>, staker: Address) -> Result<(), RegistryError> {
        let writer: Address = env.storage().instance().get(&DataKey::Writer).unwrap();
        writer.require_auth();

        let key = DataKey::Wallet(wallet_hash.clone());
        if let Some(existing) = env.storage().persistent().get::<_, Registration>(&key) {
            return Err(if existing.staker == staker {
                RegistryError::AlreadyRegistered
            } else {
                RegistryError::WalletTakenByOtherStaker
            });
        }

        let skey = DataKey::StakerWallets(staker.clone());
        let mut wallets: Vec<BytesN<32>> = env.storage().persistent().get(&skey).unwrap_or(vec![&env]);
        if wallets.len() >= MAX_WALLETS_PER_STAKER {
            return Err(RegistryError::StakerLimitReached);
        }

        let registered_at = env.ledger().timestamp();
        env.storage().persistent().set(&key, &Registration { staker: staker.clone(), registered_at });
        wallets.push_back(wallet_hash.clone());
        env.storage().persistent().set(&skey, &wallets);
        bump_wallet(&env, &wallet_hash);
        bump_staker(&env, &staker);
        bump_instance(&env);

        WalletRegistered { staker, wallet_hash, registered_at }.publish(&env);
        Ok(())
    }

    /// Pool only: called by the pool's governance when a writer change
    /// executes (`GovChange::RegistryWriter`).
    pub fn set_writer(env: Env, new_writer: Address) {
        let pool: Address = env.storage().instance().get(&DataKey::Pool).unwrap();
        pool.require_auth();
        let old_writer: Address = env.storage().instance().get(&DataKey::Writer).unwrap();
        env.storage().instance().set(&DataKey::Writer, &new_writer);
        bump_instance(&env);
        WriterChanged { old_writer, new_writer }.publish(&env);
    }

    /// Permissionless: keep a registration and its staker's list from being
    /// archived. Archived entries are restorable, never deleted, but this
    /// keeps reads cheap.
    pub fn extend_ttl(env: Env, wallet_hash: BytesN<32>) -> Result<(), RegistryError> {
        let reg: Registration = env
            .storage()
            .persistent()
            .get(&DataKey::Wallet(wallet_hash.clone()))
            .ok_or(RegistryError::NotRegistered)?;
        bump_wallet(&env, &wallet_hash);
        bump_staker(&env, &reg.staker);
        Ok(())
    }

    // -- views --

    pub fn get_registration(env: Env, wallet_hash: BytesN<32>) -> Option<Registration> {
        env.storage().persistent().get(&DataKey::Wallet(wallet_hash))
    }

    pub fn get_wallets(env: Env, staker: Address) -> Vec<BytesN<32>> {
        env.storage().persistent().get(&DataKey::StakerWallets(staker)).unwrap_or(vec![&env])
    }

    /// Registered AND its staker has a live stake (`pool.is_eligible`).
    pub fn is_covered(env: Env, wallet_hash: BytesN<32>) -> bool {
        let reg: Registration = match env.storage().persistent().get(&DataKey::Wallet(wallet_hash)) {
            Some(r) => r,
            None => return false,
        };
        let pool: Address = env.storage().instance().get(&DataKey::Pool).unwrap();
        let args: Vec<Val> = vec![&env, reg.staker.into_val(&env)];
        env.invoke_contract::<bool>(&pool, &Symbol::new(&env, "is_eligible"), args)
    }

    pub fn get_writer(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Writer).unwrap()
    }

    pub fn get_pool(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Pool).unwrap()
    }
}

#[cfg(test)]
mod test;
