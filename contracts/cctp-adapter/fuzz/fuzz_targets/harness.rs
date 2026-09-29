//! Copy of the adapter test harness (`src/test/mod.rs` + the owner-auth helpers
//! in `src/test/flows.rs`), which is `#[cfg(test)]` and so not importable here.
//! Moving it out of the contract crate would touch contract `src/` before the
//! audit freeze, so it is duplicated instead. Keep the two in step.
//!
//! Circle's REAL CCTP v2 contracts (WASM fetched from Stellar testnet
//! 2026-09-22, see testdata/circle/README.md), our own test attesters.

use cctp_common::{
    burn, contract_bytes32, domain, header, OwnerKey, ATA_PROGRAM, BURN_MESSAGE_VERSION, FINALITY_STANDARD,
    MESSAGE_VERSION, SPL_TOKEN_PROGRAM,
};
use ed25519_dalek::Signer as _;
use k256::ecdsa::SigningKey;
use safu_account::OwnerSignature;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::xdr::{
    Hash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs, Limits, ScAddress,
    ScSymbol, ScVal, SorobanAddressCredentials, SorobanAuthorizationEntry, SorobanAuthorizedFunction,
    SorobanAuthorizedInvocation, SorobanCredentials, VecM, WriteXdr,
};
use soroban_sdk::{vec, Address, Bytes, BytesN, Env, IntoVal, TryFromVal, Val, Vec};

// Hand-written clients: `contractimport!` rejects these WASMs (duplicate
// `RoleError` in their specs). Fields mirror `stellar contract info interface`.
pub mod mt {
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
    #[allow(dead_code)]
    pub trait MessageTransmitter {
        fn receive_message(env: Env, caller: Address, message: Bytes, attestation: Bytes) -> bool;
        fn is_nonce_used(env: Env, nonce: BytesN<32>) -> bool;
    }
}
pub mod tmm {
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
    #[allow(dead_code)]
    pub trait TokenMessengerMinter {
        fn link_token_pair(env: Env, local_token: Address, remote_domain: u32, remote_token: BytesN<32>);
        fn set_token_decimal_config(env: Env, local_token: Address, local_decimals: u32, canonical_decimals: u32);
        fn get_token_decimal_config(env: Env, local_token: Address) -> Option<TokenDecimalConfig>;
        fn set_swap_minter_config(env: Env, local_token: Address, swap_minter: Address, allow_asset: Address);
        fn set_max_burn_amount_per_message(env: Env, local_token: Address, burn_limit_per_message: i128);
    }
}
pub mod fta {
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
    #[allow(dead_code)]
    pub trait FiatTokenAdmin {
        fn configure_minter(env: Env, minter: Address, allow_asset: Address);
    }
}

const MAX_MESSAGE_BODY_SIZE: u32 = 8192;
const SIGNATURE_THRESHOLD: u32 = 2;
const USDC_LOCAL_DECIMALS: u32 = 7;
const USDC_CANONICAL_DECIMALS: u32 = 6;
const SEPOLIA_TOKEN_MESSENGER: [u8; 20] = hex20("8fe6b999dc680ccfdd5bf7eb0974218be2542daa");
const SEPOLIA_USDC: [u8; 20] = hex20("1c7d4b196cb0c7b01d743fbc6116a902379c7238");
const SOLANA_TOKEN_MESSENGER: [u8; 32] =
    hex32("a65fc81d0fefa8860cb3b83f089b0224be8a6687b7ae49f594c0b9b4d7e93893");
const SOLANA_USDC: [u8; 32] = [0x5a; 32];
const MINT_ALLOWANCE: i128 = 1_000_000_000_000_000;
const AUTH_VALIDITY_LEDGERS: u32 = 100;

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

/// Solana deposit hook data for `owner` (same as `src/test/mod.rs`): its USDC
/// token account for the test mint with `bump`, then the bump.
pub fn sol_payout_hook(env: &Env, owner: [u8; 32], bump: u8) -> std::vec::Vec<u8> {
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

pub fn pad20(a: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(a);
    out
}

pub fn eth_address(env: &Env, key: &SigningKey) -> [u8; 20] {
    let pt = key.verifying_key().to_encoded_point(false);
    let h = env.crypto().keccak256(&Bytes::from_slice(env, &pt.as_bytes()[1..])).to_array();
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    out
}

pub struct Circle<'a> {
    pub mt: mt::Client<'a>,
    pub tmm: tmm::Client<'a>,
    pub usdc: TokenClient<'a>,
    attesters: std::vec::Vec<SigningKey>,
}

pub fn setup_circle(env: &Env) -> Circle<'_> {
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(env);

    let mut attesters: std::vec::Vec<SigningKey> =
        (1u8..=2).map(|i| SigningKey::from_slice(&[i + 40; 32]).unwrap()).collect();
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
    fta::Client::new(env, &fta_id).configure_minter(&tmm_id, &allow_sac.address());
    StellarAssetClient::new(env, &allow_sac.address()).mint(&tmm_id, &MINT_ALLOWANCE);

    let tmm_c = tmm::Client::new(env, &tmm_id);
    tmm_c.link_token_pair(&usdc_sac.address(), &domain::ETHEREUM, &BytesN::from_array(env, &pad20(&SEPOLIA_USDC)));
    tmm_c.link_token_pair(&usdc_sac.address(), &domain::SOLANA, &BytesN::from_array(env, &SOLANA_USDC));
    tmm_c.set_token_decimal_config(&usdc_sac.address(), &USDC_LOCAL_DECIMALS, &USDC_CANONICAL_DECIMALS);
    tmm_c.set_swap_minter_config(&usdc_sac.address(), &fta_id, &allow_sac.address());
    tmm_c.set_max_burn_amount_per_message(&usdc_sac.address(), &MINT_ALLOWANCE);

    Circle {
        mt: mt::Client::new(env, &mt_id),
        tmm: tmm_c,
        usdc: TokenClient::new(env, &usdc_sac.address()),
        attesters,
    }
}

impl Circle<'_> {
    /// A home-chain -> Stellar burn message as raw bytes (so the caller can
    /// tamper with it), `amount` in canonical (6) decimals.
    #[allow(clippy::too_many_arguments)]
    pub fn inbound_message(
        &self,
        source: u32,
        nonce: [u8; 32],
        mint_recipient: &Address,
        destination_caller: [u8; 32],
        sender: [u8; 32],
        amount: u64,
    ) -> std::vec::Vec<u8> {
        let (messenger, burn_token) = if source == domain::SOLANA {
            (SOLANA_TOKEN_MESSENGER, SOLANA_USDC)
        } else {
            (pad20(&SEPOLIA_TOKEN_MESSENGER), pad20(&SEPOLIA_USDC))
        };
        let mut m = std::vec::Vec::<u8>::new();
        m.extend_from_slice(&MESSAGE_VERSION.to_be_bytes());
        m.extend_from_slice(&source.to_be_bytes());
        m.extend_from_slice(&domain::STELLAR.to_be_bytes());
        m.extend_from_slice(&nonce);
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
        m
    }

    /// Every attester signs keccak256(message), sorted by address.
    pub fn attest(&self, env: &Env, message: &Bytes) -> Bytes {
        let digest = env.crypto().keccak256(message).to_array();
        let mut out = std::vec::Vec::<u8>::new();
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

pub enum Owner {
    Evm(SigningKey),
    Sol(ed25519_dalek::SigningKey),
}

impl Owner {
    pub fn evm(seed: u8) -> Self {
        Owner::Evm(SigningKey::from_slice(&[seed; 32]).unwrap())
    }
    pub fn sol(seed: u8) -> Self {
        Owner::Sol(ed25519_dalek::SigningKey::from_bytes(&[seed; 32]))
    }
    pub fn is_evm(&self) -> bool {
        matches!(self, Owner::Evm(_))
    }
    pub fn domain(&self) -> u32 {
        match self {
            Owner::Evm(_) => domain::ETHEREUM,
            Owner::Sol(_) => domain::SOLANA,
        }
    }
    /// CCTP `messageSender` of this owner's burn.
    pub fn sender32(&self, env: &Env) -> [u8; 32] {
        match self {
            Owner::Evm(k) => pad20(&eth_address(env, k)),
            Owner::Sol(k) => k.verifying_key().to_bytes(),
        }
    }
    pub fn key(&self, env: &Env) -> OwnerKey {
        match self {
            Owner::Evm(k) => OwnerKey::Evm(BytesN::from_array(env, &eth_address(env, k))),
            Owner::Sol(k) => OwnerKey::Solana(BytesN::from_array(env, &k.verifying_key().to_bytes())),
        }
    }
    fn sign(&self, env: &Env, payload: &[u8; 32]) -> OwnerSignature {
        match self {
            Owner::Evm(k) => {
                let mut msg = std::vec::Vec::from(&b"\x19Ethereum Signed Message:\n32"[..]);
                msg.extend_from_slice(payload);
                let digest = env.crypto().keccak256(&Bytes::from_slice(env, &msg)).to_array();
                let (sig, rec) = k.sign_prehash_recoverable(&digest).unwrap();
                let mut raw = [0u8; 65];
                raw[..64].copy_from_slice(&sig.to_bytes());
                raw[64] = 27 + rec.to_byte();
                OwnerSignature::Evm(BytesN::from_array(env, &raw))
            }
            Owner::Sol(k) => OwnerSignature::Solana(BytesN::from_array(env, &k.sign(payload).to_bytes())),
        }
    }
}

fn sc_address(env: &Env, a: &Address) -> ScAddress {
    match ScVal::try_from_val(env, &a.to_val()).unwrap() {
        ScVal::Address(x) => x,
        _ => unreachable!(),
    }
}

/// A real Soroban auth entry for `account`, signed by `signer` (the owner, or
/// a thief in negative cases).
pub fn owner_auth(
    env: &Env,
    account: &Address,
    signer: &Owner,
    nonce: i64,
    contract: &Address,
    fn_name: &str,
    args: Vec<Val>,
) -> SorobanAuthorizationEntry {
    let expiry = env.ledger().sequence() + AUTH_VALIDITY_LEDGERS;
    let sc_args: std::vec::Vec<ScVal> = args.iter().map(|v| ScVal::try_from_val(env, &v).unwrap()).collect();
    let invocation = SorobanAuthorizedInvocation {
        function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
            contract_address: sc_address(env, contract),
            function_name: ScSymbol(fn_name.try_into().unwrap()),
            args: sc_args.try_into().unwrap(),
        }),
        sub_invocations: VecM::default(),
    };
    let preimage = HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
        network_id: Hash(env.ledger().network_id().to_array()),
        nonce,
        signature_expiration_ledger: expiry,
        invocation: invocation.clone(),
    });
    let payload = env
        .crypto()
        .sha256(&Bytes::from_slice(env, &preimage.to_xdr(Limits::none()).unwrap()))
        .to_array();
    let sig: Val = signer.sign(env, &payload).into_val(env);
    SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address: sc_address(env, account),
            nonce,
            signature_expiration_ledger: expiry,
            signature: ScVal::try_from_val(env, &sig).unwrap(),
        }),
        root_invocation: invocation,
    }
}
