// SPDX-License-Identifier: Apache-2.0

//! SAFU CCTP adapter (v1, Phase 3).
//!
//! Receives USDC burned on a user's home chain (Ethereum, Solana) and puts it
//! into that user's own SAFU account on Stellar, which stakes it.
//!
//! Flow, one transaction, permissionless (a relayer pays the fee):
//! 1. The user burns USDC at home with this adapter as BOTH `mintRecipient`
//!    and `destinationCaller`, so nobody else can complete the message.
//! 2. `mint_and_stake(message, attestation)` checks both, calls Circle's
//!    `receive_message`, and measures what was minted by balance delta.
//! 3. The owner is the burner (`messageSender`). A Solana burn also carries
//!    the owner's USDC token account in hook data (`SOLANA_HOOK_LEN`): it is
//!    kept only if it recomputes as the owner's own associated token account
//!    for the burned mint, and the account saves the first valid one for good.
//!    Missing or wrong hook data never blocks the deposit.
//! 4. The owner's account is deployed on first use at a fixed address
//!    (`account_salt`), the USDC moves into it, and the account stakes it
//!    (or backs the pool, see below). If the pool refuses (bounds, cap,
//!    already staked, paused) the USDC waits in the account. A CCTP mint is
//!    never reversed, so nothing here may strand it.
//!
//! Mode: each adapter is deployed as either a staking adapter or a backing
//! adapter (`DepositMode`). SAFU runs one of each; the user's burn names the
//! adapter, so the user's own signature picks the intent. A user who does
//! both gets two separate accounts (different deployer, different address).
//!
//! No admin, no upgrade: every address and the mode are fixed at construction.
//!
//! Integration rules for the backend and frontend: `docs/CCTP.md`.

#![no_std]

use cctp_common::{
    account_salt, contract_bytes32, owner_from_sender, owner_to_bytes32, parse_inbound,
    solana_payout_account, DepositMode, OwnerKey, SUPPORTED_HOME_DOMAINS,
};
use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, token,
    Address, Bytes, BytesN, Env,
};

const LEDGERS_PER_DAY: u32 = 17_280;
const BUMP_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
const BUMP_TO: u32 = 120 * LEDGERS_PER_DAY;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AdapterError {
    /// Not a CCTP v2 burn message.
    MalformedMessage = 1,
    /// `destinationCaller` is not this adapter.
    WrongDestinationCaller = 2,
    /// `mintRecipient` is not this adapter.
    WrongMintRecipient = 3,
    /// The source chain is not a supported home chain.
    UnsupportedDomain = 4,
    /// The burner address is not valid for its chain.
    UnsupportedSender = 5,
    /// Circle's `receive_message` returned false.
    ReceiveFailed = 6,
    /// The message minted nothing to this adapter.
    NothingMinted = 7,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Transmitter,
    Messenger,
    Usdc,
    Pool,
    AccountWasm,
    Mode,
    Account(BytesN<32>),
}

#[contractevent]
pub struct AccountCreated {
    #[topic]
    pub account: Address,
    pub home_domain: u32,
    pub owner: OwnerKey,
}

#[contractevent]
pub struct MintForwarded {
    #[topic]
    pub account: Address,
    pub amount: i128,
    pub source_domain: u32,
}

/// Circle's MessageTransmitterV2 (the call this adapter makes).
#[contractclient(name = "TransmitterClient")]
pub trait Transmitter {
    fn receive_message(env: Env, caller: Address, message: Bytes, attestation: Bytes) -> bool;
}

/// The per-user SAFU account (`safu-account`), as seen by the adapter.
#[contractclient(name = "SafuAccountClient")]
pub trait SafuAccount {
    fn on_mint(env: Env, payout_account: Option<BytesN<32>>);
}

#[contract]
pub struct CctpAdapter;

fn get(env: &Env, key: DataKey) -> Address {
    env.storage().instance().get(&key).unwrap()
}

fn bump(env: &Env) {
    env.storage().instance().extend_ttl(BUMP_THRESHOLD, BUMP_TO);
}

#[contractimpl]
impl CctpAdapter {
    /// `transmitter` = Circle MessageTransmitterV2, `messenger` = Circle
    /// TokenMessengerMinterV2 (passed on to each account for outbound burns),
    /// `usdc` = the USDC SAC CCTP mints, `pool` = SAFU protection-pool,
    /// `account_wasm` = the uploaded `safu-account` WASM hash, `mode` = what
    /// deposits through this adapter are for.
    pub fn __constructor(
        env: Env,
        transmitter: Address,
        messenger: Address,
        usdc: Address,
        pool: Address,
        account_wasm: BytesN<32>,
        mode: DepositMode,
    ) {
        let s = env.storage().instance();
        s.set(&DataKey::Transmitter, &transmitter);
        s.set(&DataKey::Messenger, &messenger);
        s.set(&DataKey::Usdc, &usdc);
        s.set(&DataKey::Pool, &pool);
        s.set(&DataKey::AccountWasm, &account_wasm);
        s.set(&DataKey::Mode, &mode);
        bump(&env);
    }

    /// Completes a CCTP burn into the burner's SAFU account and stakes it.
    /// Returns the account address.
    pub fn mint_and_stake(
        env: Env,
        message: Bytes,
        attestation: Bytes,
    ) -> Result<Address, AdapterError> {
        let me = env.current_contract_address();
        let me32 = contract_bytes32(&me).ok_or(AdapterError::MalformedMessage)?;
        let burn = parse_inbound(&message).ok_or(AdapterError::MalformedMessage)?;

        // Checked BEFORE receive_message: a rejected call leaves the message
        // unconsumed, so it can still be completed once the problem is fixed.
        if burn.destination_caller != me32 {
            return Err(AdapterError::WrongDestinationCaller);
        }
        if burn.mint_recipient != me32 {
            return Err(AdapterError::WrongMintRecipient);
        }
        if !SUPPORTED_HOME_DOMAINS.contains(&burn.source_domain) {
            return Err(AdapterError::UnsupportedDomain);
        }
        let owner = owner_from_sender(&env, burn.source_domain, &burn.message_sender)
            .ok_or(AdapterError::UnsupportedSender)?;

        let usdc = token::Client::new(&env, &get(&env, DataKey::Usdc));
        let before = usdc.balance(&me);
        let ok = TransmitterClient::new(&env, &get(&env, DataKey::Transmitter))
            .receive_message(&me, &message, &attestation);
        if !ok {
            return Err(AdapterError::ReceiveFailed);
        }
        let minted = usdc.balance(&me) - before;
        if minted <= 0 {
            return Err(AdapterError::NothingMinted);
        }

        // Checked only after a successful receive: that is what proves
        // `burn_token` is the real USDC mint.
        let payout_account = match &owner {
            OwnerKey::Solana(key) => solana_payout_account(&env, key, &burn.burn_token, &burn.hook_data),
            OwnerKey::Evm(_) => None,
        };

        let account = Self::ensure_account(&env, burn.source_domain, &owner);
        usdc.transfer(&me, &account, &minted);
        SafuAccountClient::new(&env, &account).on_mint(&payout_account);

        MintForwarded { account: account.clone(), amount: minted, source_domain: burn.source_domain }
            .publish(&env);
        bump(&env);
        Ok(account)
    }

    /// The account address for a home-chain owner, deployed or not. The
    /// frontend shows it before the user burns.
    pub fn account_address(env: Env, home_domain: u32, owner: OwnerKey) -> Address {
        env.deployer()
            .with_current_contract(account_salt(&env, home_domain, &owner))
            .deployed_address()
    }

    /// The account address for a raw CCTP `messageSender`, if valid.
    pub fn account_for_sender(env: Env, home_domain: u32, sender: BytesN<32>) -> Option<Address> {
        let owner = owner_from_sender(&env, home_domain, &sender)?;
        Some(Self::account_address(env, home_domain, owner))
    }

    pub fn owner_bytes32(env: Env, owner: OwnerKey) -> BytesN<32> {
        owner_to_bytes32(&env, &owner)
    }

    pub fn usdc(env: Env) -> Address {
        get(&env, DataKey::Usdc)
    }

    pub fn pool(env: Env) -> Address {
        get(&env, DataKey::Pool)
    }

    pub fn transmitter(env: Env) -> Address {
        get(&env, DataKey::Transmitter)
    }

    pub fn messenger(env: Env) -> Address {
        get(&env, DataKey::Messenger)
    }

    pub fn mode(env: Env) -> DepositMode {
        env.storage().instance().get(&DataKey::Mode).unwrap()
    }

    /// Permissionless TTL extension.
    pub fn extend_ttl(env: Env) {
        bump(&env);
    }
}

impl CctpAdapter {
    fn ensure_account(env: &Env, home_domain: u32, owner: &OwnerKey) -> Address {
        let salt = account_salt(env, home_domain, owner);
        let key = DataKey::Account(salt.clone());
        if let Some(existing) = env.storage().persistent().get::<_, Address>(&key) {
            env.storage().persistent().extend_ttl(&key, BUMP_THRESHOLD, BUMP_TO);
            return existing;
        }
        let wasm: BytesN<32> = env.storage().instance().get(&DataKey::AccountWasm).unwrap();
        let account = env.deployer().with_current_contract(salt).deploy_v2(
            wasm,
            (
                env.current_contract_address(),
                get(env, DataKey::Pool),
                get(env, DataKey::Usdc),
                get(env, DataKey::Messenger),
                owner.clone(),
                home_domain,
                Self::mode(env.clone()),
            ),
        );
        env.storage().persistent().set(&key, &account);
        env.storage().persistent().extend_ttl(&key, BUMP_THRESHOLD, BUMP_TO);
        AccountCreated { account: account.clone(), home_domain, owner: owner.clone() }.publish(env);
        account
    }
}

#[cfg(test)]
mod test;
