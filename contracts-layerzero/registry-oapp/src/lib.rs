#![no_std]
//! SAFU covered-wallet registry, delivered over LayerZero.
//!
//! A registration is made on the source chain (Ethereum / Solana) and relayed here. The message is
//! exactly 65 bytes: `staker_hash(32) ++ chain_id(1) ++ wallet_hash(32)`. The commitment stored is
//! `sha256(message)`, mirroring `beneficiary_hash` in protection-pool.
//!
//! Trust model: this contract only RECORDS what the LayerZero endpoint has verified. Replay is the
//! endpoint's job (`clear` runs before `__lz_receive`); the trusted sender is pinned per source
//! chain by `set_peer`. The off-chain registry stays the primary gate; a commitment here only adds
//! a verification label and a tamper check, it never blocks a claim on its own.
//!
//! Own storage only. This contract never touches protection-pool state.

use endpoint_v2::Origin;
use oapp::oapp_receiver::LzReceiveInternal;
use soroban_sdk::{
    contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address, Bytes, BytesN, Env,
};

#[cfg(test)]
mod test;

pub const MESSAGE_LEN: u32 = 65;
pub const CHAIN_ETH: u8 = 1;
pub const CHAIN_SOLANA: u8 = 2;

const DAY_IN_LEDGERS: u32 = 17_280;
pub const BUMP_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
pub const BUMP_TO: u32 = 120 * DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RegistryError {
    BadPayloadLength = 1,
    UnknownChain = 2,
}

#[contracttype]
#[derive(Clone)]
pub enum RegKey {
    Commitment(BytesN<32>),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub src_eid: u32,
    pub ledger_timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WalletRegistered {
    pub commitment: BytesN<32>,
    pub src_eid: u32,
    pub chain_id: u32,
}

#[oapp_macros::oapp]
#[common_macros::lz_contract]
pub struct RegistryOApp;

impl LzReceiveInternal for RegistryOApp {
    fn __lz_receive(
        env: &Env,
        origin: &Origin,
        _guid: &BytesN<32>,
        message: &Bytes,
        _extra_data: &Bytes,
        _executor: &Address,
        _value: i128,
    ) {
        if message.len() != MESSAGE_LEN {
            panic_with_error!(env, RegistryError::BadPayloadLength);
        }
        // chain_id sits at byte 32, between the two 32-byte hashes.
        let chain_id = message.get(32).unwrap_or(0);
        if chain_id != CHAIN_ETH && chain_id != CHAIN_SOLANA {
            panic_with_error!(env, RegistryError::UnknownChain);
        }

        let commitment: BytesN<32> = env.crypto().sha256(message).into();
        let key = RegKey::Commitment(commitment.clone());
        // First registration wins: a re-delivery must not move the original timestamp.
        if !env.storage().persistent().has(&key) {
            let reg = Registration { src_eid: origin.src_eid, ledger_timestamp: env.ledger().timestamp() };
            env.storage().persistent().set(&key, &reg);
            WalletRegistered { commitment, src_eid: origin.src_eid, chain_id: chain_id as u32 }.publish(env);
        }
        env.storage().persistent().extend_ttl(&key, BUMP_THRESHOLD, BUMP_TO);
    }
}

#[contractimpl]
impl RegistryOApp {
    pub fn __constructor(env: &Env, owner: &Address, endpoint: &Address) {
        oapp::oapp_core::init_ownable_oapp::<Self>(env, owner, endpoint, owner);
    }

    /// Ledger timestamp at which this commitment was first recorded, or `None`.
    pub fn is_registered(env: &Env, commitment: BytesN<32>) -> Option<u64> {
        env.storage()
            .persistent()
            .get::<_, Registration>(&RegKey::Commitment(commitment))
            .map(|r| r.ledger_timestamp)
    }

    /// Extend a registration's TTL. Permissionless: anyone can already extend any entry's TTL
    /// at the protocol level, so gating this adds nothing. Returns false if nothing is stored.
    pub fn extend_registration_ttl(env: &Env, commitment: BytesN<32>) -> bool {
        let key = RegKey::Commitment(commitment);
        if !env.storage().persistent().has(&key) {
            return false;
        }
        env.storage().persistent().extend_ttl(&key, BUMP_THRESHOLD, BUMP_TO);
        true
    }

    /// Source endpoint the commitment arrived from, or `None`.
    pub fn source_eid(env: &Env, commitment: BytesN<32>) -> Option<u32> {
        env.storage().persistent().get::<_, Registration>(&RegKey::Commitment(commitment)).map(|r| r.src_eid)
    }
}
