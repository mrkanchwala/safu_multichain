#![cfg(test)]
//! Harness: Circle's REAL CCTP v2 contracts (WASM fetched from Stellar testnet
//! 2026-09-22, see testdata/circle/README.md) wired exactly like testnet, with
//! our own test attesters so messages can be signed locally.

extern crate std;

use cctp_common::{
    burn, contract_bytes32, domain, header, ATA_PROGRAM, BURN_MESSAGE_VERSION, FINALITY_STANDARD, MESSAGE_VERSION,
    SPL_TOKEN_PROGRAM,
};
use k256::ecdsa::SigningKey;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{vec, Address, Bytes, BytesN, Env};
use std::vec::Vec as StdVec;

// Hand-written clients: `contractimport!` rejects these WASMs because their
// specs list `RoleError` more than once. Struct fields mirror
// `stellar contract info interface` output exactly (contracttype structs
// encode by field name).
mod mt {
    use soroban_sdk::{contractclient, contracttype, Address, Bytes, BytesN, Env, Vec};
    pub const WASM: &[u8] = include_bytes!("../../testdata/circle/message_transmitter.wasm");
    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct MessageTransmitterV2ContractInitParams {
        pub admin: Address,
        pub attester_manager: Address,
        pub attesters: Vec<BytesN<20>>,
        pub local_domain: u32,
        pub max_message_body_size: u32,
        pub owner: Address,
        pub pauser: Address,
        pub rescuer: Address,
        pub signature_threshold: u32,
        pub version: u32,
    }
    #[contractclient(name = "Client")]
    #[allow(dead_code)] // only the generated Client is used
    pub trait MessageTransmitter {
        fn receive_message(env: Env, caller: Address, message: Bytes, attestation: Bytes) -> bool;
        fn is_nonce_used(env: Env, nonce: BytesN<32>) -> bool;
    }
}
mod tmm {
    use soroban_sdk::{contractclient, contracttype, Address, BytesN, Env, Vec};
    pub const WASM: &[u8] = include_bytes!("../../testdata/circle/token_messenger_minter.wasm");
    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct TokenMessengerMinterV2ContractInitParams {
        pub admin: Address,
        pub denylister: Address,
        pub fee_recipient: Address,
        pub message_body_version: u32,
        pub message_transmitter: Address,
        pub min_fee_controller: Address,
        pub owner: Address,
        pub pauser: Address,
        pub remote_domains: Vec<u32>,
        pub remote_token_messengers: Vec<BytesN<32>>,
        pub rescuer: Address,
        pub token_controller: Address,
    }
    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct TokenDecimalConfig {
        pub canonical_decimals: u32,
        pub local_decimals: u32,
    }
    #[contractclient(name = "Client")]
    #[allow(dead_code)] // only the generated Client is used
    pub trait TokenMessengerMinter {
        fn link_token_pair(env: Env, local_token: Address, remote_domain: u32, remote_token: BytesN<32>);
        fn set_token_decimal_config(env: Env, local_token: Address, local_decimals: u32, canonical_decimals: u32);
        fn get_token_decimal_config(env: Env, local_token: Address) -> Option<TokenDecimalConfig>;
        fn set_swap_minter_config(env: Env, local_token: Address, swap_minter: Address, allow_asset: Address);
        fn set_max_burn_amount_per_message(env: Env, local_token: Address, burn_limit_per_message: i128);
    }
}
mod fta {
    use soroban_sdk::{contractclient, contracttype, Address, Env};
    pub const WASM: &[u8] = include_bytes!("../../testdata/circle/fiat_token_admin.wasm");
    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct FiatTokenAdminContractInitParams {
        pub admin: Address,
        pub blocklister: Address,
        pub mint_asset: Address,
        pub minter_asset_controller: Address,
        pub owner: Address,
        pub pauser: Address,
    }
    #[contractclient(name = "Client")]
    #[allow(dead_code)] // only the generated Client is used
    pub trait FiatTokenAdmin {
        fn configure_minter(env: Env, minter: Address, allow_asset: Address);
    }
}

/// Live testnet MessageTransmitter settings, read 2026-09-22.
const MAX_MESSAGE_BODY_SIZE: u32 = 8192;
const SIGNATURE_THRESHOLD: u32 = 2;
/// Live testnet decimal config for Stellar USDC (local 7, canonical 6).
const USDC_LOCAL_DECIMALS: u32 = 7;
const USDC_CANONICAL_DECIMALS: u32 = 6;
/// Sepolia TokenMessengerV2 and USDC (domain 0), as registered on testnet.
const SEPOLIA_TOKEN_MESSENGER: [u8; 20] = hex20("8fe6b999dc680ccfdd5bf7eb0974218be2542daa");
const SEPOLIA_USDC: [u8; 20] = hex20("1c7d4b196cb0c7b01d743fbc6116a902379c7238");
/// Solana TokenMessengerMinterV2 (domain 5), as registered on testnet.
const SOLANA_TOKEN_MESSENGER: [u8; 32] =
    hex32("a65fc81d0fefa8860cb3b83f089b0224be8a6687b7ae49f594c0b9b4d7e93893");
/// Stand-in Solana USDC mint: `link_token_pair` only needs the pair to match.
const SOLANA_USDC: [u8; 32] = [0x5a; 32];

const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}
/// Allowance given to the minter; far above any test amount.
const MINT_ALLOWANCE: i128 = 1_000_000_000_000_000;

const fn hex20(s: &str) -> [u8; 20] {
    let b = s.as_bytes();
    let mut out = [0u8; 20];
    let mut i = 0;
    while i < 20 {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        _ => c - b'a' + 10,
    }
}

pub(crate) fn pad20(a: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(a);
    out
}

pub(crate) fn eth_address(env: &Env, key: &SigningKey) -> [u8; 20] {
    let pt = key.verifying_key().to_encoded_point(false);
    let h = env.crypto().keccak256(&Bytes::from_slice(env, &pt.as_bytes()[1..])).to_array();
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    out
}

pub(crate) struct Circle<'a> {
    pub mt: mt::Client<'a>,
    pub tmm: tmm::Client<'a>,
    pub usdc: TokenClient<'a>,
    pub usdc_admin: StellarAssetClient<'a>,
    attesters: StdVec<SigningKey>,
}

pub(crate) fn setup_circle(env: &Env) -> Circle<'_> {
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(env);

    let mut attesters: StdVec<SigningKey> = (1u8..=2)
        .map(|i| SigningKey::from_slice(&[i + 40; 32]).unwrap())
        .collect();
    attesters.sort_by_key(|k| eth_address(env, k));
    let mut attester_addrs = vec![env];
    for k in &attesters {
        attester_addrs.push_back(BytesN::from_array(env, &eth_address(env, k)));
    }

    let mt_id = env.register(
        mt::WASM,
        (mt::MessageTransmitterV2ContractInitParams {
            admin: admin.clone(),
            attester_manager: admin.clone(),
            attesters: attester_addrs,
            local_domain: domain::STELLAR,
            max_message_body_size: MAX_MESSAGE_BODY_SIZE,
            owner: admin.clone(),
            pauser: admin.clone(),
            rescuer: admin.clone(),
            signature_threshold: SIGNATURE_THRESHOLD,
            version: MESSAGE_VERSION,
        },),
    );

    let issuer = Address::generate(env);
    let usdc_sac = env.register_stellar_asset_contract_v2(issuer.clone());
    let allow_sac = env.register_stellar_asset_contract_v2(issuer);

    let tmm_id = env.register(
        tmm::WASM,
        (tmm::TokenMessengerMinterV2ContractInitParams {
            admin: admin.clone(),
            denylister: admin.clone(),
            fee_recipient: admin.clone(),
            message_body_version: BURN_MESSAGE_VERSION,
            message_transmitter: mt_id.clone(),
            min_fee_controller: admin.clone(),
            owner: admin.clone(),
            pauser: admin.clone(),
            remote_domains: vec![env, domain::ETHEREUM, domain::SOLANA],
            remote_token_messengers: vec![
                env,
                BytesN::from_array(env, &pad20(&SEPOLIA_TOKEN_MESSENGER)),
                BytesN::from_array(env, &SOLANA_TOKEN_MESSENGER),
            ],
            rescuer: admin.clone(),
            token_controller: admin.clone(),
        },),
    );

    let fta_id = env.register(
        fta::WASM,
        (fta::FiatTokenAdminContractInitParams {
            admin: admin.clone(),
            blocklister: admin.clone(),
            mint_asset: usdc_sac.address(),
            minter_asset_controller: admin.clone(),
            owner: admin.clone(),
            pauser: admin.clone(),
        },),
    );
    StellarAssetClient::new(env, &usdc_sac.address()).set_admin(&fta_id);
    let fta_c = fta::Client::new(env, &fta_id);
    fta_c.configure_minter(&tmm_id, &allow_sac.address());
    StellarAssetClient::new(env, &allow_sac.address()).mint(&tmm_id, &MINT_ALLOWANCE);

    let tmm_c = tmm::Client::new(env, &tmm_id);
    tmm_c.link_token_pair(
        &usdc_sac.address(),
        &domain::ETHEREUM,
        &BytesN::from_array(env, &pad20(&SEPOLIA_USDC)),
    );
    tmm_c.link_token_pair(&usdc_sac.address(), &domain::SOLANA, &BytesN::from_array(env, &SOLANA_USDC));
    tmm_c.set_token_decimal_config(&usdc_sac.address(), &USDC_LOCAL_DECIMALS, &USDC_CANONICAL_DECIMALS);
    tmm_c.set_swap_minter_config(&usdc_sac.address(), &fta_id, &allow_sac.address());
    tmm_c.set_max_burn_amount_per_message(&usdc_sac.address(), &MINT_ALLOWANCE);

    Circle {
        mt: mt::Client::new(env, &mt_id),
        tmm: tmm_c,
        usdc: TokenClient::new(env, &usdc_sac.address()),
        usdc_admin: StellarAssetClient::new(env, &usdc_sac.address()),
        attesters,
    }
}

/// Solana deposit hook data for `owner`: its associated USDC token account for
/// the test mint (`SOLANA_USDC`) with `bump`, then the bump. The derivation
/// itself is proven against `@solana/web3.js` vectors in `cctp-common`.
pub fn sol_payout_hook(env: &Env, owner: [u8; 32], bump: u8) -> StdVec<u8> {
    let mut pre = Bytes::from_array(env, &owner);
    pre.extend_from_array(&SPL_TOKEN_PROGRAM);
    pre.extend_from_array(&SOLANA_USDC);
    pre.push_back(bump);
    pre.extend_from_array(&ATA_PROGRAM);
    pre.extend_from_slice(b"ProgramDerivedAddress");
    let mut hook = env.crypto().sha256(&pre).to_array().to_vec();
    hook.push(bump);
    hook
}

impl<'a> Circle<'a> {
    /// A home-chain -> Stellar burn message, `amount` in canonical (6)
    /// decimals. `source` is Ethereum or Solana.
    #[allow(clippy::too_many_arguments)]
    pub fn inbound_message(
        &self,
        env: &Env,
        source: u32,
        nonce: u8,
        mint_recipient: &Address,
        destination_caller: [u8; 32],
        sender: [u8; 32],
        amount: u64,
    ) -> Bytes {
        self.inbound_message_with_hook(env, source, nonce, mint_recipient, destination_caller, sender, amount, &[])
    }

    /// `inbound_message` with hook data appended after the fixed body.
    #[allow(clippy::too_many_arguments)]
    pub fn inbound_message_with_hook(
        &self,
        env: &Env,
        source: u32,
        nonce: u8,
        mint_recipient: &Address,
        destination_caller: [u8; 32],
        sender: [u8; 32],
        amount: u64,
        hook: &[u8],
    ) -> Bytes {
        let (messenger, burn_token) = if source == domain::SOLANA {
            (SOLANA_TOKEN_MESSENGER, SOLANA_USDC)
        } else {
            (pad20(&SEPOLIA_TOKEN_MESSENGER), pad20(&SEPOLIA_USDC))
        };
        let mut m = StdVec::<u8>::new();
        m.extend_from_slice(&MESSAGE_VERSION.to_be_bytes());
        m.extend_from_slice(&source.to_be_bytes());
        m.extend_from_slice(&domain::STELLAR.to_be_bytes());
        m.extend_from_slice(&[nonce; 32]);
        m.extend_from_slice(&messenger);
        m.extend_from_slice(&contract_bytes32(&self.tmm.address).unwrap().to_array());
        m.extend_from_slice(&destination_caller);
        m.extend_from_slice(&FINALITY_STANDARD.to_be_bytes());
        m.extend_from_slice(&FINALITY_STANDARD.to_be_bytes());
        assert_eq!(m.len() as u32, header::BODY);
        m.extend_from_slice(&BURN_MESSAGE_VERSION.to_be_bytes());
        m.extend_from_slice(&burn_token);
        m.extend_from_slice(&contract_bytes32(mint_recipient).unwrap().to_array());
        m.extend_from_slice(&u256(amount));
        m.extend_from_slice(&sender);
        m.extend_from_slice(&u256(0)); // max fee
        m.extend_from_slice(&u256(0)); // fee executed
        m.extend_from_slice(&u256(0)); // expiration block
        assert_eq!(m.len() as u32, header::BODY + burn::HOOK_DATA);
        m.extend_from_slice(hook);
        Bytes::from_slice(env, &m)
    }

    /// Attestation: every attester signs keccak256(message), sorted by address.
    pub fn attest(&self, env: &Env, message: &Bytes) -> Bytes {
        let digest = env.crypto().keccak256(message).to_array();
        let mut out = StdVec::<u8>::new();
        for k in &self.attesters {
            let (sig, rec) = k.sign_prehash_recoverable(&digest).unwrap();
            out.extend_from_slice(&sig.to_bytes());
            out.push(27 + rec.to_byte());
        }
        Bytes::from_slice(env, &out)
    }

    /// 10^(local - canonical), read from Circle's live config.
    pub fn scale(&self) -> i128 {
        let c = self.tmm.get_token_decimal_config(&self.usdc.address).unwrap();
        10i128.pow(c.local_decimals - c.canonical_decimals)
    }
}

fn u256(v: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&v.to_be_bytes());
    out
}

/// Probe: the harness reproduces Circle's real inbound path (attested message
/// -> TokenMessengerMinter -> FiatTokenAdmin swap_mint -> USDC), with the
/// 6 -> 7 decimal scaling, before any SAFU contract is involved.
#[test]
fn probe_circle_harness_mints_real_usdc() {
    let env = Env::default();
    let c = setup_circle(&env);
    let recipient = env.register(crate::CctpAdapter, (
        c.mt.address.clone(), c.tmm.address.clone(), c.usdc.address.clone(),
        Address::generate(&env), BytesN::from_array(&env, &[0u8; 32]),
        cctp_common::DepositMode::Stake,
    ));
    let caller = Address::generate(&env);
    let amount: u64 = 25_000_000; // 25 USDC, canonical decimals
    let msg = c.inbound_message(&env, domain::ETHEREUM, 1, &recipient, [0u8; 32], pad20(&[9u8; 20]), amount);
    let att = c.attest(&env, &msg);
    assert!(c.mt.receive_message(&caller, &msg, &att));
    assert_eq!(c.usdc.balance(&recipient), amount as i128 * c.scale());
}

mod flows;
