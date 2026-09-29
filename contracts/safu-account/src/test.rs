#![cfg(test)]
//! `__check_auth` in isolation: only the owner's own home-chain signature
//! passes, and only for the calls this account allows.

extern crate std;

use crate::{AccountError, OwnerSignature, SafuAccountContract, EIP191_PREFIX_32, EVM_V_OFFSET, POOL_DIRECT_FNS};
use cctp_common::{domain, DepositMode, OwnerKey};
use ed25519_dalek::Signer as _;
use k256::ecdsa::SigningKey;
use soroban_sdk::auth::{Context, ContractContext, ContractExecutable, CreateContractHostFnContext};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Bytes, BytesN, Env, IntoVal, Symbol, Val, Vec};

struct T {
    account: Address,
    pool: Address,
    usdc: Address,
}

fn eth_address(env: &Env, key: &SigningKey) -> [u8; 20] {
    let pt = key.verifying_key().to_encoded_point(false);
    let h = env.crypto().keccak256(&Bytes::from_slice(env, &pt.as_bytes()[1..])).to_array();
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    out
}

fn deploy(env: &Env, owner: OwnerKey, home: u32) -> T {
    let pool = Address::generate(env);
    let usdc = Address::generate(env);
    let account = env.register(
        SafuAccountContract,
        (Address::generate(env), pool.clone(), usdc.clone(), Address::generate(env), owner, home, DepositMode::Stake),
    );
    T { account, pool, usdc }
}

fn evm_key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).unwrap()
}

fn evm_account(env: &Env, key: &SigningKey) -> T {
    deploy(env, OwnerKey::Evm(BytesN::from_array(env, &eth_address(env, key))), domain::ETHEREUM)
}

fn sol_account(env: &Env, key: &ed25519_dalek::SigningKey) -> T {
    deploy(env, OwnerKey::Solana(BytesN::from_array(env, &key.verifying_key().to_bytes())), domain::SOLANA)
}

/// personal_sign over the payload; `v_base` 27 (wire) or 0 (raw recovery id).
fn evm_sign(env: &Env, key: &SigningKey, payload: &[u8; 32], v_base: u8) -> OwnerSignature {
    let mut msg = std::vec::Vec::from(EIP191_PREFIX_32);
    msg.extend_from_slice(payload);
    let digest = env.crypto().keccak256(&Bytes::from_slice(env, &msg)).to_array();
    let (sig, rec) = key.sign_prehash_recoverable(&digest).unwrap();
    let mut raw = [0u8; 65];
    raw[..64].copy_from_slice(&sig.to_bytes());
    raw[64] = v_base + rec.to_byte();
    OwnerSignature::Evm(BytesN::from_array(env, &raw))
}

fn sol_sign(env: &Env, key: &ed25519_dalek::SigningKey, payload: &[u8; 32]) -> OwnerSignature {
    OwnerSignature::Solana(BytesN::from_array(env, &key.sign(payload).to_bytes()))
}

fn ctx(env: &Env, contract: &Address, f: &str) -> Context {
    Context::Contract(ContractContext {
        contract: contract.clone(),
        fn_name: Symbol::new(env, f),
        args: Vec::<Val>::new(env),
    })
}

type CheckResult = Result<(), Result<AccountError, soroban_sdk::InvokeError>>;

fn check(env: &Env, t: &T, payload: &[u8; 32], sig: OwnerSignature, ctxs: Vec<Context>) -> CheckResult {
    let sig: Val = sig.into_val(env);
    env.try_invoke_contract_check_auth::<AccountError>(&t.account, &BytesN::from_array(env, payload), sig, &ctxs)
}

const PAYLOAD: [u8; 32] = [0x42; 32];

// ------------------------------------------------------------------- EVM --

#[test]
fn evm_owner_signature_is_accepted_with_wire_v() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    assert_eq!(check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c), Ok(()));
}

#[test]
fn evm_owner_signature_is_accepted_with_raw_recovery_id() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    assert_eq!(check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, 0), c), Ok(()));
}

#[test]
fn evm_signature_from_another_key_is_rejected() {
    let env = Env::default();
    let t = evm_account(&env, &evm_key(0x11));
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let forged = evm_sign(&env, &evm_key(0x66), &PAYLOAD, EVM_V_OFFSET);
    assert_eq!(check(&env, &t, &PAYLOAD, forged, c), Err(Ok(AccountError::NotOwner)));
}

#[test]
fn evm_signature_over_a_different_payload_is_rejected() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let other = evm_sign(&env, &k, &[0x43; 32], EVM_V_OFFSET);
    assert_eq!(check(&env, &t, &PAYLOAD, other, c), Err(Ok(AccountError::NotOwner)));
}

#[test]
fn evm_signature_without_eip191_prefix_is_rejected() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let (sig, rec) = k.sign_prehash_recoverable(&PAYLOAD).unwrap(); // raw, unprefixed
    let mut raw = [0u8; 65];
    raw[..64].copy_from_slice(&sig.to_bytes());
    raw[64] = EVM_V_OFFSET + rec.to_byte();
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let r = check(&env, &t, &PAYLOAD, OwnerSignature::Evm(BytesN::from_array(&env, &raw)), c);
    assert_eq!(r, Err(Ok(AccountError::NotOwner)));
}

#[test]
fn evm_bad_recovery_id_is_rejected() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let OwnerSignature::Evm(sig) = evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET) else { unreachable!() };
    let mut raw = sig.to_array();
    raw[64] = EVM_V_OFFSET + 2;
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let r = check(&env, &t, &PAYLOAD, OwnerSignature::Evm(BytesN::from_array(&env, &raw)), c);
    assert_eq!(r, Err(Ok(AccountError::BadRecoveryId)));
}

// ---------------------------------------------------------------- Solana --

#[test]
fn solana_owner_signature_is_accepted() {
    let env = Env::default();
    let k = ed25519_dalek::SigningKey::from_bytes(&[0x22; 32]);
    let t = sol_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    assert_eq!(check(&env, &t, &PAYLOAD, sol_sign(&env, &k, &PAYLOAD), c), Ok(()));
}

#[test]
fn solana_signature_from_another_key_is_rejected() {
    let env = Env::default();
    let t = sol_account(&env, &ed25519_dalek::SigningKey::from_bytes(&[0x22; 32]));
    let forged = sol_sign(&env, &ed25519_dalek::SigningKey::from_bytes(&[0x66; 32]), &PAYLOAD);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    assert!(check(&env, &t, &PAYLOAD, forged, c).is_err());
}

#[test]
fn signature_kind_must_match_the_owner_chain() {
    let env = Env::default();
    let sol = ed25519_dalek::SigningKey::from_bytes(&[0x22; 32]);
    let t = sol_account(&env, &sol);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let evm_sig = evm_sign(&env, &evm_key(0x11), &PAYLOAD, EVM_V_OFFSET);
    assert_eq!(check(&env, &t, &PAYLOAD, evm_sig, c), Err(Ok(AccountError::WrongSignatureKind)));

    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home")];
    let r = check(&env, &t, &PAYLOAD, sol_sign(&env, &sol, &PAYLOAD), c);
    assert_eq!(r, Err(Ok(AccountError::WrongSignatureKind)));
}

// ------------------------------------------------------------- allowlist --

#[test]
fn owner_may_authorise_every_own_function_and_the_direct_pool_calls() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let mut c = vec![&env];
    for f in [
        "send_home", "withdraw_home", "exit_home", "claim_home", "stake_held",
        "back_held", "request_back_withdrawal", "cancel_back_withdrawal", "complete_back_withdrawal_home",
        "claim_yield_home", "claim_backer_yield_home",
    ] {
        c.push_back(ctx(&env, &t.account, f));
    }
    for f in POOL_DIRECT_FNS {
        c.push_back(ctx(&env, &t.pool, f));
    }
    assert_eq!(check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c), Ok(()));
}

#[test]
fn owner_signature_cannot_authorise_other_pool_calls() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    // Direct withdraw / set_beneficiary on the pool would let a signed payload
    // route money somewhere other than home.
    for f in [
        "withdraw", "set_beneficiary", "emergency_exit", "claim_stream", "stake",
        "back", "request_backer_withdrawal", "cancel_backer_withdrawal", "complete_backer_withdrawal",
        // r3: yield goes home only through the account's own *_home functions.
        "claim_yield", "claim_backer_yield",
    ] {
        let c = vec![&env, ctx(&env, &t.pool, f)];
        let r = check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c);
        assert_eq!(r, Err(Ok(AccountError::ContextNotAllowed)), "{f}");
    }
}

#[test]
fn owner_signature_cannot_authorise_token_calls() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    for f in ["transfer", "approve", "burn"] {
        let c = vec![&env, ctx(&env, &t.usdc, f)];
        let r = check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c);
        assert_eq!(r, Err(Ok(AccountError::ContextNotAllowed)), "{f}");
    }
}

#[test]
fn owner_signature_cannot_authorise_an_unrelated_contract() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &Address::generate(&env), "approve_claim")];
    let r = check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c);
    assert_eq!(r, Err(Ok(AccountError::ContextNotAllowed)));
}

#[test]
fn owner_signature_cannot_authorise_contract_creation() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![
        &env,
        Context::CreateContractHostFn(CreateContractHostFnContext {
            executable: ContractExecutable::Wasm(BytesN::from_array(&env, &[1u8; 32])),
            salt: BytesN::from_array(&env, &[2u8; 32]),
        }),
    ];
    let r = check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c);
    assert_eq!(r, Err(Ok(AccountError::ContextNotAllowed)));
}

#[test]
fn one_disallowed_context_fails_the_whole_signature() {
    let env = Env::default();
    let k = evm_key(0x11);
    let t = evm_account(&env, &k);
    let c = vec![&env, ctx(&env, &t.account, "send_home"), ctx(&env, &t.usdc, "transfer")];
    let r = check(&env, &t, &PAYLOAD, evm_sign(&env, &k, &PAYLOAD, EVM_V_OFFSET), c);
    assert_eq!(r, Err(Ok(AccountError::ContextNotAllowed)));
}

// ---------------------------------------------------------------- on_mint --

#[test]
fn only_the_adapter_can_trigger_on_mint() {
    let env = Env::default();
    let t = evm_account(&env, &evm_key(0x11));
    let client = crate::SafuAccountContractClient::new(&env, &t.account);
    // No auth mocked: the adapter has not authorised this call.
    assert!(client.try_on_mint(&None).is_err());
}
