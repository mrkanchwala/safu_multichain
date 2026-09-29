//! v1 (2026-09-23, pre-audit hardening): fuzz target for `safu-account`'s
//! `__check_auth`, the only thing standing between a home-chain signature and
//! the user's money.
//!
//! Each input picks an account (EVM- or Solana-owned), a payload, a signature
//! (the owner's real one, a mutated one, another key's, raw bytes, the wrong
//! chain's) and a list of call contexts. A model predicts the exact result.
//!
//! Invariants:
//!   1. NOTHING passes unless it is the owner's own signature over this exact
//!      payload AND every context is allowed (a call on the account itself, or
//!      `approve_claim` / `revoke_approval` on the pool).
//!   2. Wrong chain's signature kind -> WrongSignatureKind, before anything else.
//!   3. EVM `v` outside {0,1,27,28} -> BadRecoveryId.
//!   4. A valid owner signature with any disallowed context -> ContextNotAllowed.
//!   5. Another key's valid EVM signature -> NotOwner (never a pass).
//!
//! Run: `cargo +nightly fuzz run fuzz_check_auth -- -max_total_time=600`

#![no_main]

use arbitrary::Arbitrary;
use cctp_common::{domain, DepositMode, OwnerKey};
use ed25519_dalek::Signer as _;
use k256::ecdsa::SigningKey;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::auth::{Context, ContractContext, ContractExecutable, CreateContractHostFnContext};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, BytesN, Env, IntoVal, Symbol, Val, Vec};

use safu_account::{AccountError, OwnerSignature, SafuAccountContract};

const EIP191_PREFIX_32: &[u8] = b"\x19Ethereum Signed Message:\n32";
const EVM_OWNER_SEED: u8 = 0x11;
const SOL_OWNER_SEED: u8 = 0x22;
const MAX_CONTEXTS: usize = 8;

/// Every owner-facing function on the account (all allowed).
const ACCOUNT_FNS: [&str; 11] = [
    "send_home", "withdraw_home", "exit_home", "claim_home", "stake_held",
    "back_held", "request_back_withdrawal", "cancel_back_withdrawal", "complete_back_withdrawal_home",
    // r3 (2026-09-29)
    "claim_yield_home", "claim_backer_yield_home",
];
/// Pool functions: the first two are allowed directly, the rest must not be.
const POOL_FNS: [&str; 13] = [
    "approve_claim", "revoke_approval",
    "withdraw", "set_beneficiary", "emergency_exit", "claim_stream", "stake",
    "back", "request_backer_withdrawal", "cancel_backer_withdrawal", "complete_backer_withdrawal",
    // r3: yield goes home only through the account's own *_home functions.
    "claim_yield", "claim_backer_yield",
];
const POOL_ALLOWED: usize = 2;
const TOKEN_FNS: [&str; 4] = ["transfer", "approve", "burn", "transfer_from"];

#[derive(Arbitrary, Debug)]
enum Sig {
    /// The owner's real signature over the payload (EVM: wire v or raw id).
    Owner { raw_v: bool },
    /// The owner's real signature over a different payload.
    OwnerOtherPayload { other: [u8; 32] },
    /// The owner's r||s with an arbitrary v (EVM only; Solana -> Owner).
    OwnerWithV { v: u8 },
    /// The owner's signature with one bit flipped.
    OwnerBitFlip { byte: u8, bit: u8 },
    /// Another key on the same chain, signing the same payload.
    OtherKey { seed: u8 },
    /// The owner's key, but the other chain's signature kind.
    WrongKind,
    RawEvm { bytes: [u8; 65] },
    RawSol { bytes: [u8; 64] },
}

#[derive(Arbitrary, Debug)]
enum Ctx {
    OwnFn(u8),
    OwnFnAnyName(u8),
    PoolFn(u8),
    TokenFn(u8),
    UnrelatedContract(u8),
    CreateContract,
}

#[derive(Arbitrary, Debug)]
struct Input {
    solana: bool,
    payload: [u8; 32],
    sig: Sig,
    contexts: std::vec::Vec<Ctx>,
}

fn eth_address(env: &Env, key: &SigningKey) -> [u8; 20] {
    let pt = key.verifying_key().to_encoded_point(false);
    let h = env.crypto().keccak256(&Bytes::from_slice(env, &pt.as_bytes()[1..])).to_array();
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    out
}

/// personal_sign: returns r||s and the recovery id (0/1).
fn evm_sign(env: &Env, key: &SigningKey, payload: &[u8; 32]) -> ([u8; 64], u8) {
    let mut msg = std::vec::Vec::from(EIP191_PREFIX_32);
    msg.extend_from_slice(payload);
    let digest = env.crypto().keccak256(&Bytes::from_slice(env, &msg)).to_array();
    let (sig, rec) = key.sign_prehash_recoverable(&digest).unwrap();
    let mut rs = [0u8; 64];
    rs.copy_from_slice(&sig.to_bytes());
    (rs, rec.to_byte())
}

fn evm_sig(env: &Env, rs: &[u8; 64], v: u8) -> OwnerSignature {
    let mut raw = [0u8; 65];
    raw[..64].copy_from_slice(rs);
    raw[64] = v;
    OwnerSignature::Evm(BytesN::from_array(env, &raw))
}

fn sol_sig(env: &Env, key: &ed25519_dalek::SigningKey, payload: &[u8; 32]) -> OwnerSignature {
    OwnerSignature::Solana(BytesN::from_array(env, &key.sign(payload).to_bytes()))
}

/// Non-zero scalar well below the curve order, never the owner's.
fn other_seed(seed: u8, owner: u8) -> u8 {
    let s = 1 + seed % 200;
    if s == owner { s + 1 } else { s }
}

fn ctx(env: &Env, contract: &Address, f: &str) -> Context {
    Context::Contract(ContractContext {
        contract: contract.clone(),
        fn_name: Symbol::new(env, f),
        args: Vec::<Val>::new(env),
    })
}

/// What the signature itself should produce, before contexts are checked.
enum SigVerdict {
    Valid,
    Exactly(AccountError),
    /// Must fail; the exact error is the host's (e.g. a trap in recovery).
    AnyError,
}

fuzz_target!(|input: Input| {
    let env = Env::default();
    let evm_key = SigningKey::from_slice(&[EVM_OWNER_SEED; 32]).unwrap();
    let sol_key = ed25519_dalek::SigningKey::from_bytes(&[SOL_OWNER_SEED; 32]);
    let (owner, home) = if input.solana {
        (OwnerKey::Solana(BytesN::from_array(&env, &sol_key.verifying_key().to_bytes())), domain::SOLANA)
    } else {
        (OwnerKey::Evm(BytesN::from_array(&env, &eth_address(&env, &evm_key))), domain::ETHEREUM)
    };
    let pool = Address::generate(&env);
    let usdc = Address::generate(&env);
    let account = env.register(
        SafuAccountContract,
        (Address::generate(&env), pool.clone(), usdc.clone(), Address::generate(&env), owner, home, DepositMode::Stake),
    );
    let payload = input.payload;

    let (sig, verdict) = if input.solana {
        match input.sig {
            Sig::Owner { .. } | Sig::OwnerWithV { .. } => (sol_sig(&env, &sol_key, &payload), SigVerdict::Valid),
            Sig::OwnerOtherPayload { other } => {
                let v = if other == payload { SigVerdict::Valid } else { SigVerdict::AnyError };
                (sol_sig(&env, &sol_key, &other), v)
            }
            Sig::OwnerBitFlip { byte, bit } => {
                let mut raw = sol_key.sign(&payload).to_bytes();
                raw[(byte as usize) % 64] ^= 1 << (bit % 8);
                (OwnerSignature::Solana(BytesN::from_array(&env, &raw)), SigVerdict::AnyError)
            }
            Sig::OtherKey { seed } => {
                let k = ed25519_dalek::SigningKey::from_bytes(&[other_seed(seed, SOL_OWNER_SEED); 32]);
                (sol_sig(&env, &k, &payload), SigVerdict::AnyError)
            }
            Sig::WrongKind => {
                let (rs, rec) = evm_sign(&env, &evm_key, &payload);
                (evm_sig(&env, &rs, 27 + rec), SigVerdict::Exactly(AccountError::WrongSignatureKind))
            }
            Sig::RawEvm { bytes } => (
                OwnerSignature::Evm(BytesN::from_array(&env, &bytes)),
                SigVerdict::Exactly(AccountError::WrongSignatureKind),
            ),
            Sig::RawSol { bytes } => (OwnerSignature::Solana(BytesN::from_array(&env, &bytes)), SigVerdict::AnyError),
        }
    } else {
        let (rs, rec) = evm_sign(&env, &evm_key, &payload);
        // Verdict for the owner's own r||s under a given v byte.
        let by_v = |v: u8| {
            let id = if v >= 27 { v - 27 } else { v };
            if id > 1 {
                SigVerdict::Exactly(AccountError::BadRecoveryId)
            } else if id == rec {
                SigVerdict::Valid
            } else {
                SigVerdict::AnyError
            }
        };
        match input.sig {
            Sig::Owner { raw_v } => {
                let v = if raw_v { rec } else { 27 + rec };
                (evm_sig(&env, &rs, v), SigVerdict::Valid)
            }
            Sig::OwnerOtherPayload { other } => {
                let (ors, orec) = evm_sign(&env, &evm_key, &other);
                let v = if other == payload { SigVerdict::Valid } else { SigVerdict::Exactly(AccountError::NotOwner) };
                (evm_sig(&env, &ors, 27 + orec), v)
            }
            Sig::OwnerWithV { v } => (evm_sig(&env, &rs, v), by_v(v)),
            Sig::OwnerBitFlip { byte, bit } => {
                let mut raw = [0u8; 65];
                raw[..64].copy_from_slice(&rs);
                raw[64] = 27 + rec;
                let i = (byte as usize) % 65;
                raw[i] ^= 1 << (bit % 8);
                let verdict = if i == 64 { by_v(raw[64]) } else { SigVerdict::AnyError };
                (OwnerSignature::Evm(BytesN::from_array(&env, &raw)), verdict)
            }
            Sig::OtherKey { seed } => {
                let k = SigningKey::from_slice(&[other_seed(seed, EVM_OWNER_SEED); 32]).unwrap();
                let (ors, orec) = evm_sign(&env, &k, &payload);
                (evm_sig(&env, &ors, 27 + orec), SigVerdict::Exactly(AccountError::NotOwner))
            }
            Sig::WrongKind => (
                sol_sig(&env, &sol_key, &payload),
                SigVerdict::Exactly(AccountError::WrongSignatureKind),
            ),
            Sig::RawEvm { bytes } => {
                let id = if bytes[64] >= 27 { bytes[64] - 27 } else { bytes[64] };
                let v = if id > 1 { SigVerdict::Exactly(AccountError::BadRecoveryId) } else { SigVerdict::AnyError };
                (OwnerSignature::Evm(BytesN::from_array(&env, &bytes)), v)
            }
            Sig::RawSol { bytes } => (
                OwnerSignature::Solana(BytesN::from_array(&env, &bytes)),
                SigVerdict::Exactly(AccountError::WrongSignatureKind),
            ),
        }
    };

    let mut contexts = Vec::<Context>::new(&env);
    let mut all_allowed = true;
    for c in input.contexts.into_iter().take(MAX_CONTEXTS) {
        let (context, allowed) = match c {
            Ctx::OwnFn(i) => (ctx(&env, &account, ACCOUNT_FNS[i as usize % ACCOUNT_FNS.len()]), true),
            // Any function name on the account itself is allowed by design:
            // the account's own functions each re-check require_auth.
            Ctx::OwnFnAnyName(i) => (ctx(&env, &account, POOL_FNS[i as usize % POOL_FNS.len()]), true),
            Ctx::PoolFn(i) => {
                let k = i as usize % POOL_FNS.len();
                (ctx(&env, &pool, POOL_FNS[k]), k < POOL_ALLOWED)
            }
            Ctx::TokenFn(i) => (ctx(&env, &usdc, TOKEN_FNS[i as usize % TOKEN_FNS.len()]), false),
            Ctx::UnrelatedContract(i) => (
                ctx(&env, &Address::generate(&env), POOL_FNS[i as usize % POOL_ALLOWED]),
                false,
            ),
            Ctx::CreateContract => (
                Context::CreateContractHostFn(CreateContractHostFnContext {
                    executable: ContractExecutable::Wasm(BytesN::from_array(&env, &[1u8; 32])),
                    salt: BytesN::from_array(&env, &[2u8; 32]),
                }),
                false,
            ),
        };
        all_allowed &= allowed;
        contexts.push_back(context);
    }

    let sig_val: Val = sig.into_val(&env);
    let got = env.try_invoke_contract_check_auth::<AccountError>(
        &account,
        &BytesN::from_array(&env, &payload),
        sig_val,
        &contexts,
    );

    match verdict {
        SigVerdict::Valid if all_allowed => assert_eq!(got, Ok(()), "owner signature + allowed contexts refused"),
        SigVerdict::Valid => assert_eq!(got, Err(Ok(AccountError::ContextNotAllowed)), "disallowed context passed"),
        SigVerdict::Exactly(e) => assert_eq!(got, Err(Ok(e)), "wrong rejection"),
        SigVerdict::AnyError => assert!(got.is_err(), "AUTH BYPASS: a non-owner signature passed"),
    }
});
