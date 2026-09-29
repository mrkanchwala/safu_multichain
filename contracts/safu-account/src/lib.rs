// SPDX-License-Identifier: Apache-2.0

//! SAFU per-user CCTP account (v1, Phase 3).
//!
//! One contract per user who stakes from another chain. The user never holds
//! a Stellar key: the account is the staker in the pool, and it accepts only
//! its owner's home-chain signature (EVM secp256k1 or Solana Ed25519).
//!
//! Rules (founder, 2026-09-22):
//! - Money only ever leaves toward the owner's own home-chain address
//!   (`send_home`). No other destination, no admin, no upgrade. No call
//!   takes a destination, so no owner signature can redirect money.
//!   Solana: CCTP mints to a USDC token account, not the wallet. The owner's
//!   deposit names it in hook data, the adapter checks it is the owner's own
//!   associated token account, and this account saves the first valid one
//!   for good (pre-audit /cso H-1, 2026-09-24: a destination signed at
//!   withdrawal was an opaque 32-byte hash, the phishing case SAFU covers).
//! - Pool payouts and withdrawals land in the account itself (beneficiary =
//!   the account), then go home.
//! - A mint the pool refuses (bounds, cap, already staked, paused) is held in
//!   the account, never lost. The owner can stake/back it later or send it
//!   home.
//! - Mode (`DepositMode`, fixed at creation by the adapter that deployed it):
//!   a Stake account stakes arriving USDC, a Back account backs the pool with
//!   it. Backer withdrawals go through the pool's notice period and return
//!   to this account, then home, like everything else.
//!
//! Owner signatures:
//! - EVM: `personal_sign` of the 32-byte Soroban auth payload, i.e.
//!   keccak256("\x19Ethereum Signed Message:\n32" || payload), 65-byte r||s||v.
//! - Solana: Ed25519 over the 32-byte payload (wallet `signMessage`).
//!
//! Replay protection comes from Soroban's auth payload (nonce + expiry). The
//! payload's preimage format (legacy or `address_v2`) is the host's business;
//! this contract only sees the 32-byte hash.
//!
//! Integration rules for the backend and frontend: `docs/CCTP.md`.

#![no_std]

use cctp_common::{owner_to_bytes32, DepositMode, OwnerKey, FINALITY_STANDARD};
use soroban_sdk::auth::{
    Context, ContractContext, CustomAccountInterface, InvokerContractAuthEntry,
    SubContractInvocation,
};
use soroban_sdk::crypto::Hash;
use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    symbol_short, token, vec, Address, Bytes, BytesN, Env, IntoVal, Symbol, Vec,
};

const LEDGERS_PER_DAY: u32 = 17_280;
const BUMP_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
const BUMP_TO: u32 = 120 * LEDGERS_PER_DAY;

/// EIP-191 prefix for a 32-byte message.
pub(crate) const EIP191_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";
/// EVM `v` is 27/28 on the wire; the recovery id is `v - 27`.
pub(crate) const EVM_V_OFFSET: u8 = 27;
/// Pool functions the owner may authorise directly (all other pool calls go
/// through this account's own functions, with the account as invoker).
pub(crate) const POOL_DIRECT_FNS: [&str; 2] = ["approve_claim", "revoke_approval"];

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AccountError {
    /// Signature type does not match the owner's chain.
    WrongSignatureKind = 1,
    /// The signature is not the owner's.
    NotOwner = 2,
    /// The signed call is not one this account allows.
    ContextNotAllowed = 3,
    /// Nothing bridgeable to send (below one canonical unit).
    NothingToSend = 4,
    /// Solana owner: no deposit has carried a valid USDC token account yet.
    /// Any later deposit with one (even a small one) unlocks sending home.
    PayoutAccountMissing = 5,
    /// EVM `v` is not 27/28 (or 0/1).
    BadRecoveryId = 7,
    /// Circle has no decimal config for the pool's USDC.
    DecimalConfigMissing = 8,
    /// Amount must be positive.
    AmountNotPositive = 9,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerSignature {
    /// r || s || v, v in {27, 28} (or {0, 1}).
    Evm(BytesN<65>),
    Solana(BytesN<64>),
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Adapter,
    Pool,
    Usdc,
    Messenger,
    Owner,
    HomeDomain,
    Mode,
    /// Solana owners only: their USDC token account, set once.
    PayoutAccount,
}

#[contractevent]
pub struct PayoutAccountSaved {
    pub payout_account: BytesN<32>,
}

#[contractevent]
pub struct StakedFromMint {
    pub amount: i128,
}

#[contractevent]
pub struct BackedFromMint {
    pub amount: i128,
}

#[contractevent]
pub struct Held {
    pub amount: i128,
}

#[contractevent]
pub struct SentHome {
    pub amount: i128,
    pub home_domain: u32,
    pub recipient: BytesN<32>,
}

/// Circle's TokenMessengerMinterV2 decimal config (fields match Circle's).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenDecimalConfig {
    pub canonical_decimals: u32,
    pub local_decimals: u32,
}

#[contractclient(name = "MessengerClient")]
pub trait Messenger {
    #[allow(clippy::too_many_arguments)]
    fn deposit_for_burn(
        env: Env,
        caller: Address,
        amount: i128,
        destination_domain: u32,
        mint_recipient: BytesN<32>,
        burn_token: Address,
        destination_caller: BytesN<32>,
        max_fee: i128,
        min_finality_threshold: u32,
    );
    fn get_token_decimal_config(env: Env, local_token: Address) -> Option<TokenDecimalConfig>;
    fn get_min_fee_amount(env: Env, burn_token: Address, amount: i128) -> i128;
}

/// The SAFU protection-pool calls this account makes as the staker.
#[contractclient(name = "PoolClient")]
pub trait Pool {
    fn stake(env: Env, staker: Address, amount: i128, beneficiary: Address);
    fn withdraw(env: Env, staker: Address, beneficiary: Address);
    fn emergency_exit(env: Env, staker: Address);
    /// Returns the amount streamed by this pull.
    fn claim_stream(env: Env, claim_id: BytesN<32>, beneficiary: Address) -> i128;
    fn back(env: Env, backer: Address, amount: i128);
    /// Returns the earliest completion timestamp.
    fn request_backer_withdrawal(env: Env, backer: Address, amount: i128) -> u64;
    fn cancel_backer_withdrawal(env: Env, backer: Address);
    /// Returns the amount paid to the backer.
    fn complete_backer_withdrawal(env: Env, backer: Address) -> i128;
}

#[contract]
pub struct SafuAccountContract;

fn addr(env: &Env, key: DataKey) -> Address {
    env.storage().instance().get(&key).unwrap()
}

fn owner(env: &Env) -> OwnerKey {
    env.storage().instance().get(&DataKey::Owner).unwrap()
}

fn home_domain(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::HomeDomain).unwrap()
}

fn bump(env: &Env) {
    env.storage().instance().extend_ttl(BUMP_THRESHOLD, BUMP_TO);
}

#[contractimpl]
impl SafuAccountContract {
    /// Deployed by the CCTP adapter on the owner's first deposit.
    #[allow(clippy::too_many_arguments)]
    pub fn __constructor(
        env: Env,
        adapter: Address,
        pool: Address,
        usdc: Address,
        messenger: Address,
        owner: OwnerKey,
        home_domain: u32,
        mode: DepositMode,
    ) {
        let s = env.storage().instance();
        s.set(&DataKey::Adapter, &adapter);
        s.set(&DataKey::Pool, &pool);
        s.set(&DataKey::Usdc, &usdc);
        s.set(&DataKey::Messenger, &messenger);
        s.set(&DataKey::Owner, &owner);
        s.set(&DataKey::HomeDomain, &home_domain);
        s.set(&DataKey::Mode, &mode);
        bump(&env);
    }

    /// Adapter only: stake (or back, per mode) everything the account holds,
    /// or hold it if the pool refuses. Never fails because of the pool, so a
    /// mint is never reverted. `payout_account` is already checked by the
    /// adapter to be the Solana owner's own USDC token account; the first one
    /// is kept for good, later ones are ignored.
    pub fn on_mint(env: Env, payout_account: Option<BytesN<32>>) {
        addr(&env, DataKey::Adapter).require_auth();
        let s = env.storage().instance();
        if let (Some(p), OwnerKey::Solana(_)) = (payout_account, owner(&env)) {
            if !s.has(&DataKey::PayoutAccount) {
                s.set(&DataKey::PayoutAccount, &p);
                PayoutAccountSaved { payout_account: p }.publish(&env);
            }
        }
        let amount = Self::balance(env.clone());
        match Self::mode(env.clone()) {
            DepositMode::Stake if amount > 0 && Self::try_stake(&env, amount) => {
                StakedFromMint { amount }.publish(&env)
            }
            DepositMode::Back if amount > 0 && Self::try_back(&env, amount) => {
                BackedFromMint { amount }.publish(&env)
            }
            _ => Held { amount }.publish(&env),
        }
        bump(&env);
    }

    /// Owner: back the pool with `amount` of held USDC. Fails if the pool
    /// refuses.
    pub fn back_held(env: Env, amount: i128) -> Result<(), AccountError> {
        env.current_contract_address().require_auth();
        if amount <= 0 {
            return Err(AccountError::AmountNotPositive);
        }
        Self::authorize_pool_pull(&env, amount);
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).back(&me, &amount);
        bump(&env);
        Ok(())
    }

    /// Owner: start the pool's backer notice period for `amount`. Returns
    /// the earliest completion timestamp.
    pub fn request_back_withdrawal(env: Env, amount: i128) -> u64 {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).request_backer_withdrawal(&me, &amount)
    }

    /// Owner: cancel a pending backer withdrawal.
    pub fn cancel_back_withdrawal(env: Env) {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).cancel_backer_withdrawal(&me);
    }

    /// Owner: complete a backer withdrawal after the notice period and send
    /// everything home.
    pub fn complete_back_withdrawal_home(env: Env) -> Result<i128, AccountError> {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).complete_backer_withdrawal(&me);
        Self::send_all(&env)
    }

    pub fn mode(env: Env) -> DepositMode {
        env.storage().instance().get(&DataKey::Mode).unwrap()
    }

    /// Owner: stake `amount` of held USDC. Fails if the pool refuses.
    pub fn stake_held(env: Env, amount: i128) -> Result<(), AccountError> {
        env.current_contract_address().require_auth();
        if amount <= 0 {
            return Err(AccountError::AmountNotPositive);
        }
        Self::authorize_pool_pull(&env, amount);
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).stake(&me, &amount, &me);
        bump(&env);
        Ok(())
    }

    /// Owner: withdraw the stake and send everything home.
    pub fn withdraw_home(env: Env) -> Result<i128, AccountError> {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).withdraw(&me, &me);
        Self::send_all(&env)
    }

    /// Owner: emergency exit from the pool and send everything home.
    pub fn exit_home(env: Env) -> Result<i128, AccountError> {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).emergency_exit(&me);
        Self::send_all(&env)
    }

    /// Owner: pull what has vested on a claim and send everything home.
    pub fn claim_home(env: Env, claim_id: BytesN<32>) -> Result<i128, AccountError> {
        env.current_contract_address().require_auth();
        let me = env.current_contract_address();
        PoolClient::new(&env, &addr(&env, DataKey::Pool)).claim_stream(&claim_id, &me);
        Self::send_all(&env)
    }

    /// Owner: send everything the account holds home.
    pub fn send_home(env: Env) -> Result<i128, AccountError> {
        env.current_contract_address().require_auth();
        Self::send_all(&env)
    }

    /// Solana owners: the saved USDC token account payouts go to.
    pub fn payout_account(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::PayoutAccount)
    }

    pub fn balance(env: Env) -> i128 {
        token::Client::new(&env, &addr(&env, DataKey::Usdc)).balance(&env.current_contract_address())
    }

    pub fn owner(env: Env) -> OwnerKey {
        owner(&env)
    }

    pub fn home_domain(env: Env) -> u32 {
        home_domain(&env)
    }

    pub fn adapter(env: Env) -> Address {
        addr(&env, DataKey::Adapter)
    }

    /// Permissionless TTL extension.
    pub fn extend_ttl(env: Env) {
        bump(&env);
    }
}

impl SafuAccountContract {
    /// The pool's `stake` pulls USDC from the staker one call deeper than the
    /// account invokes, so the account pre-authorises exactly that transfer.
    fn authorize_pool_pull(env: &Env, amount: i128) {
        let me = env.current_contract_address();
        let pool = addr(env, DataKey::Pool);
        env.authorize_as_current_contract(vec![
            env,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: addr(env, DataKey::Usdc),
                    fn_name: symbol_short!("transfer"),
                    args: (me, pool, amount).into_val(env),
                },
                sub_invocations: vec![env],
            }),
        ]);
    }

    fn try_stake(env: &Env, amount: i128) -> bool {
        Self::authorize_pool_pull(env, amount);
        let me = env.current_contract_address();
        PoolClient::new(env, &addr(env, DataKey::Pool))
            .try_stake(&me, &amount, &me)
            .is_ok()
    }

    fn try_back(env: &Env, amount: i128) -> bool {
        Self::authorize_pool_pull(env, amount);
        let me = env.current_contract_address();
        PoolClient::new(env, &addr(env, DataKey::Pool))
            .try_back(&me, &amount)
            .is_ok()
    }

    fn recipient(env: &Env) -> Result<BytesN<32>, AccountError> {
        match owner(env) {
            o @ OwnerKey::Evm(_) => Ok(owner_to_bytes32(env, &o)),
            OwnerKey::Solana(_) => env
                .storage()
                .instance()
                .get(&DataKey::PayoutAccount)
                .ok_or(AccountError::PayoutAccountMissing),
        }
    }

    /// Burns the whole bridgeable balance to the owner's home chain. Circle
    /// burns only down to its canonical decimals, so the remainder below one
    /// canonical unit stays here for the next send.
    fn send_all(env: &Env) -> Result<i128, AccountError> {
        let recipient = Self::recipient(env)?;
        let usdc = addr(env, DataKey::Usdc);
        let messenger = MessengerClient::new(env, &addr(env, DataKey::Messenger));
        let cfg = messenger
            .get_token_decimal_config(&usdc)
            .ok_or(AccountError::DecimalConfigMissing)?;
        let scale = 10i128.pow(cfg.local_decimals - cfg.canonical_decimals);
        let balance = token::Client::new(env, &usdc).balance(&env.current_contract_address());
        let amount = balance - balance % scale;
        if amount <= 0 {
            return Err(AccountError::NothingToSend);
        }
        let max_fee = messenger.get_min_fee_amount(&usdc, &amount);
        let me = env.current_contract_address();
        let domain = home_domain(env);
        // Circle pulls the burn with `transfer_from`, so approve exactly this
        // amount, expiring this ledger: the burn consumes all of it and no
        // allowance is left standing.
        token::Client::new(env, &usdc).approve(
            &me,
            &messenger.address,
            &amount,
            &env.ledger().sequence(),
        );
        messenger.deposit_for_burn(
            &me,
            &amount,
            &domain,
            &recipient,
            &usdc,
            &BytesN::from_array(env, &[0u8; 32]),
            &max_fee,
            &FINALITY_STANDARD,
        );
        SentHome { amount, home_domain: domain, recipient }.publish(env);
        bump(env);
        Ok(amount)
    }

    fn verify_owner(
        env: &Env,
        payload: &Hash<32>,
        signature: &OwnerSignature,
    ) -> Result<(), AccountError> {
        match (owner(env), signature) {
            (OwnerKey::Evm(expected), OwnerSignature::Evm(sig)) => {
                let raw = sig.to_array();
                let v = raw[64];
                let rec = if v >= EVM_V_OFFSET { v - EVM_V_OFFSET } else { v };
                if rec > 1 {
                    return Err(AccountError::BadRecoveryId);
                }
                let mut msg = Bytes::from_slice(env, EIP191_PREFIX_32);
                msg.append(&Bytes::from_array(env, &payload.to_array()));
                let digest = env.crypto().keccak256(&msg);
                let mut rs = [0u8; 64];
                rs.copy_from_slice(&raw[..64]);
                let pk = env
                    .crypto()
                    .secp256k1_recover(&digest, &BytesN::from_array(env, &rs), rec as u32)
                    .to_array();
                let h = env.crypto().keccak256(&Bytes::from_slice(env, &pk[1..])).to_array();
                let mut got = [0u8; 20];
                got.copy_from_slice(&h[12..]);
                if got != expected.to_array() {
                    return Err(AccountError::NotOwner);
                }
                Ok(())
            }
            (OwnerKey::Solana(pk), OwnerSignature::Solana(sig)) => {
                // Traps (fails auth) on a bad signature.
                env.crypto().ed25519_verify(&pk, &Bytes::from_array(env, &payload.to_array()), sig);
                Ok(())
            }
            _ => Err(AccountError::WrongSignatureKind),
        }
    }

    fn check_context(env: &Env, ctx: &Context) -> Result<(), AccountError> {
        let Context::Contract(c) = ctx else {
            return Err(AccountError::ContextNotAllowed);
        };
        if c.contract == env.current_contract_address() {
            return Ok(());
        }
        if c.contract == addr(env, DataKey::Pool)
            && POOL_DIRECT_FNS.iter().any(|f| c.fn_name == Symbol::new(env, f))
        {
            return Ok(());
        }
        Err(AccountError::ContextNotAllowed)
    }
}

#[contractimpl]
impl CustomAccountInterface for SafuAccountContract {
    type Signature = OwnerSignature;
    type Error = AccountError;

    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signature: OwnerSignature,
        auth_contexts: Vec<Context>,
    ) -> Result<(), AccountError> {
        Self::verify_owner(&env, &signature_payload, &signature)?;
        for ctx in auth_contexts.iter() {
            Self::check_context(&env, &ctx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test;
