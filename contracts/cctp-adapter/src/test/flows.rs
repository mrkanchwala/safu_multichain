//! End-to-end CCTP flows on the REAL stack: Circle's contracts, the SAFU pool,
//! and the `safu-account` WASM the adapter deploys. Owner actions carry REAL
//! signed Soroban auth entries (EVM personal_sign / Solana Ed25519), never
//! mocked auth: `mock_auths` would replace the account contract itself.

use super::*;
use crate::{AdapterError, CctpAdapterClient};
use cctp_common::{owner_to_bytes32, DepositMode, OwnerKey};
use ed25519_dalek::Signer as _;
use protection_pool::{ProtectionPoolClient, SettingKey};
use safu_account::{AccountError, OwnerSignature, SafuAccountContractClient};
use soroban_sdk::testutils::Ledger;
use soroban_sdk::xdr::{
    Hash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs, Limits,
    ScAddress, ScSymbol, ScVal, SorobanAddressCredentials, SorobanAuthorizationEntry,
    SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials, VecM, WriteXdr,
};
use soroban_sdk::{IntoVal, TryFromVal, Val, Vec};

// Every SAFU contract runs as its release WASM, as on-chain, so metered cost
// is realistic. Build first: `stellar contract build` (see the crate README).
const ACCOUNT_WASM: &[u8] =
    include_bytes!("../../../target/wasm32v1-none/release/safu_account.wasm");
const ADAPTER_WASM: &[u8] =
    include_bytes!("../../../target/wasm32v1-none/release/cctp_adapter.wasm");
const POOL_WASM: &[u8] =
    include_bytes!("../../../target/wasm32v1-none/release/protection_pool.wasm");

/// 100,000 USDC at Stellar's 7 decimals.
const POOL_CAP: i128 = 1_000_000_000_000;
/// Basis-point denominator for the pool's stake-bound settings.
const BPS: i128 = 10_000;
/// Plain Stellar stakers, so the pool has capital beyond the CCTP stake.
const OTHER_STAKERS: u32 = 20;
const LEDGERS_PER_DAY: u32 = 17_280;
const SECONDS_PER_LEDGER: u64 = 5;
/// Well past the pool's 90-day time gate.
const PAST_TIME_GATE: u32 = 180 * LEDGERS_PER_DAY;
const AUTH_VALIDITY_LEDGERS: u32 = 100;

struct Stack<'a> {
    c: Circle<'a>,
    pool: ProtectionPoolClient<'a>,
    adapter: CctpAdapterClient<'a>,
    backer_adapter: CctpAdapterClient<'a>,
    oracle: Address,
    oracle_key: ed25519_dalek::SigningKey,
}

enum Owner {
    Evm(SigningKey),
    Sol(ed25519_dalek::SigningKey),
}

impl Owner {
    fn evm(seed: u8) -> Self {
        Owner::Evm(SigningKey::from_slice(&[seed; 32]).unwrap())
    }
    fn sol(seed: u8) -> Self {
        Owner::Sol(ed25519_dalek::SigningKey::from_bytes(&[seed; 32]))
    }
    fn domain(&self) -> u32 {
        match self {
            Owner::Evm(_) => domain::ETHEREUM,
            Owner::Sol(_) => domain::SOLANA,
        }
    }
    /// CCTP `messageSender` of this owner's burn.
    fn sender32(&self, env: &Env) -> [u8; 32] {
        match self {
            Owner::Evm(k) => pad20(&eth_address(env, k)),
            Owner::Sol(k) => k.verifying_key().to_bytes(),
        }
    }
    fn key(&self, env: &Env) -> OwnerKey {
        match self {
            Owner::Evm(k) => OwnerKey::Evm(BytesN::from_array(env, &eth_address(env, k))),
            Owner::Sol(k) => OwnerKey::Solana(BytesN::from_array(env, &k.verifying_key().to_bytes())),
        }
    }
    fn sign(&self, env: &Env, payload: &[u8; 32]) -> OwnerSignature {
        match self {
            Owner::Evm(k) => {
                let mut msg = StdVec::from(&b"\x19Ethereum Signed Message:\n32"[..]);
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

fn setup_stack(env: &Env) -> Stack<'_> {
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
        li.min_persistent_entry_ttl = 10_000_000;
        li.min_temp_entry_ttl = 10_000_000;
        li.max_entry_ttl = 20_000_000;
    });
    let c = setup_circle(env);
    let admin = Address::generate(env);
    let oracle = Address::generate(env);
    let oracle_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let pool_id = env.register(
        POOL_WASM,
        (
            admin,
            oracle.clone(),
            BytesN::from_array(env, &oracle_key.verifying_key().to_bytes()),
            Address::generate(env),
            Address::generate(env), // guardian
            c.usdc.address.clone(),
            POOL_CAP,
        ),
    );
    let pool = ProtectionPoolClient::new(env, &pool_id);
    let wasm = env.deployer().upload_contract_wasm(ACCOUNT_WASM);
    let adapter_args = |mode: DepositMode| {
        (c.mt.address.clone(), c.tmm.address.clone(), c.usdc.address.clone(), pool_id.clone(), wasm.clone(), mode)
    };
    let adapter_id = env.register(ADAPTER_WASM, adapter_args(DepositMode::Stake));
    let backer_id = env.register(ADAPTER_WASM, adapter_args(DepositMode::Back));
    let s = Stack {
        c,
        pool,
        adapter: CctpAdapterClient::new(env, &adapter_id),
        backer_adapter: CctpAdapterClient::new(env, &backer_id),
        oracle,
        oracle_key,
    };
    let (_, max) = bounds(&s);
    for _ in 0..OTHER_STAKERS {
        let st = Address::generate(env);
        s.c.usdc_admin.mint(&st, &max);
        s.pool.stake(&st, &max, &st);
    }
    // The Budget counter accumulates across every call in this Env; reset it
    // so it does not bill setup to the test. Per-transaction network limits
    // (InvocationResourceLimits) stay enforced by the SDK on every call.
    env.cost_estimate().budget().reset_unlimited();
    s
}

/// (min, max) stake from the pool's live settings.
fn bounds(s: &Stack) -> (i128, i128) {
    (
        POOL_CAP * s.pool.get_setting(&SettingKey::MinStakeBps) / BPS,
        POOL_CAP * s.pool.get_setting(&SettingKey::MaxStakeBps) / BPS,
    )
}

fn mid_stake(s: &Stack) -> i128 {
    let (min, max) = bounds(s);
    let mid = (min + max) / 2;
    mid - mid % s.c.scale()
}

type BridgeResult =
    Result<Result<Address, soroban_sdk::ConversionError>, Result<AdapterError, soroban_sdk::InvokeError>>;

/// Burns `amount` (Stellar decimals) at home and relays it through `adapter`.
fn bridge_via(
    env: &Env,
    s: &Stack,
    adapter: &CctpAdapterClient,
    source: u32,
    nonce: u8,
    sender: [u8; 32],
    amount: i128,
) -> BridgeResult {
    let me32 = contract_bytes32(&adapter.address).unwrap().to_array();
    let canonical = (amount / s.c.scale()) as u64;
    bridge_with_hook(env, s, adapter, source, nonce, sender, amount, &[])
}

/// `bridge_via` with hook data on the burn (Solana: the payout account).
#[allow(clippy::too_many_arguments)]
fn bridge_with_hook(
    env: &Env,
    s: &Stack,
    adapter: &CctpAdapterClient,
    source: u32,
    nonce: u8,
    sender: [u8; 32],
    amount: i128,
    hook: &[u8],
) -> BridgeResult {
    let me32 = contract_bytes32(&adapter.address).unwrap().to_array();
    let canonical = (amount / s.c.scale()) as u64;
    let msg = s.c.inbound_message_with_hook(env, source, nonce, &adapter.address, me32, sender, canonical, hook);
    let att = s.c.attest(env, &msg);
    adapter.try_mint_and_stake(&msg, &att)
}

/// Through the staking adapter.
fn bridge_in(env: &Env, s: &Stack, source: u32, nonce: u8, sender: [u8; 32], amount: i128) -> BridgeResult {
    bridge_via(env, s, &s.adapter, source, nonce, sender, amount)
}

fn sc_address(env: &Env, a: &Address) -> ScAddress {
    match ScVal::try_from_val(env, &a.to_val()).unwrap() {
        ScVal::Address(x) => x,
        _ => unreachable!(),
    }
}

/// A real Soroban auth entry for `account`, signed by its home-chain owner.
fn owner_auth(
    env: &Env,
    account: &Address,
    owner: &Owner,
    nonce: i64,
    contract: &Address,
    fn_name: &str,
    args: Vec<Val>,
) -> SorobanAuthorizationEntry {
    let expiry = env.ledger().sequence() + AUTH_VALIDITY_LEDGERS;
    let sc_args: StdVec<ScVal> = args.iter().map(|v| ScVal::try_from_val(env, &v).unwrap()).collect();
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
    let sig: Val = owner.sign(env, &payload).into_val(env);
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

fn staked(s: &Stack, account: &Address) -> i128 {
    s.pool.get_stake(account).map(|r| if r.withdrawn { 0 } else { r.amount }).unwrap_or(0)
}

// ---------------------------------------------------------------- inbound --

#[test]
fn evm_bridge_in_deploys_account_and_stakes_with_no_mocked_auth() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    env.set_auths(&[]); // strict: nothing mocked from here
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();

    assert_eq!(account, s.adapter.account_address(&owner.domain(), &owner.key(&env)));
    assert_eq!(staked(&s, &account), amount);
    assert_eq!(s.c.usdc.balance(&s.adapter.address), 0);
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.balance(), 0);
    assert_eq!(acct.owner(), owner.key(&env));
    assert_eq!(acct.home_domain(), domain::ETHEREUM);
    assert_eq!(acct.adapter(), s.adapter.address);
}

#[test]
fn solana_bridge_in_deploys_solana_owned_account() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::sol(0x22);
    let amount = mid_stake(&s);
    env.set_auths(&[]);
    let account = bridge_in(&env, &s, owner.domain(), 2, owner.sender32(&env), amount).unwrap().unwrap();
    assert_eq!(staked(&s, &account), amount);
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.owner(), owner.key(&env));
    assert_eq!(acct.home_domain(), domain::SOLANA);
}

#[test]
fn same_owner_on_two_chains_gets_two_accounts() {
    let env = Env::default();
    let s = setup_stack(&env);
    let sender = Owner::evm(0x11).sender32(&env);
    let eth = s.adapter.account_for_sender(&domain::ETHEREUM, &BytesN::from_array(&env, &sender));
    let sol = s.adapter.account_for_sender(&domain::SOLANA, &BytesN::from_array(&env, &sender));
    assert_ne!(eth, sol);
}

#[test]
fn second_bridge_in_while_staked_is_held_in_the_same_account() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let a1 = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let a2 = bridge_in(&env, &s, owner.domain(), 2, owner.sender32(&env), amount).unwrap().unwrap();
    assert_eq!(a1, a2);
    assert_eq!(staked(&s, &a1), amount); // one stake per address: second is refused
    assert_eq!(SafuAccountContractClient::new(&env, &a1).balance(), amount);
}

#[test]
fn over_max_is_held_then_owner_stakes_part_of_it() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let (_, max) = bounds(&s);
    let amount = max + s.c.scale() * 1_000;
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    assert_eq!(staked(&s, &account), 0);
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.balance(), amount);

    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "stake_held", (max,).into_val(&env))]);
    acct.stake_held(&max);
    assert_eq!(staked(&s, &account), max);
    assert_eq!(acct.balance(), amount - max);
}

#[test]
fn rejects_wrong_destination_caller_and_leaves_message_unused() {
    let env = Env::default();
    let s = setup_stack(&env);
    let sender = Owner::evm(0x11).sender32(&env);
    let amount = (mid_stake(&s) / s.c.scale()) as u64;
    let nonce = 9u8;
    let msg = s.c.inbound_message(&env, domain::ETHEREUM, nonce, &s.adapter.address, [0u8; 32], sender, amount);
    let att = s.c.attest(&env, &msg);
    assert_eq!(s.adapter.try_mint_and_stake(&msg, &att), Err(Ok(AdapterError::WrongDestinationCaller)));
    assert!(!s.c.mt.is_nonce_used(&BytesN::from_array(&env, &[nonce; 32])));
}

#[test]
fn rejects_wrong_mint_recipient() {
    let env = Env::default();
    let s = setup_stack(&env);
    let me32 = contract_bytes32(&s.adapter.address).unwrap().to_array();
    let sender = Owner::evm(0x11).sender32(&env);
    let amount = (mid_stake(&s) / s.c.scale()) as u64;
    let msg = s.c.inbound_message(&env, domain::ETHEREUM, 3, &s.pool.address, me32, sender, amount);
    let att = s.c.attest(&env, &msg);
    assert_eq!(s.adapter.try_mint_and_stake(&msg, &att), Err(Ok(AdapterError::WrongMintRecipient)));
}

#[test]
fn rejects_unsupported_home_domain() {
    let env = Env::default();
    let s = setup_stack(&env);
    let unsupported = domain::STELLAR; // any domain outside SUPPORTED_HOME_DOMAINS
    let r = bridge_in(&env, &s, unsupported, 4, Owner::evm(0x11).sender32(&env), mid_stake(&s));
    assert_eq!(r, Err(Ok(AdapterError::UnsupportedDomain)));
}

#[test]
fn rejects_evm_sender_that_is_not_a_padded_address() {
    let env = Env::default();
    let s = setup_stack(&env);
    let r = bridge_in(&env, &s, domain::ETHEREUM, 5, [0xab; 32], mid_stake(&s));
    assert_eq!(r, Err(Ok(AdapterError::UnsupportedSender)));
}

#[test]
fn rejects_truncated_message() {
    let env = Env::default();
    let s = setup_stack(&env);
    let short = Bytes::from_slice(&env, &[0u8; 64]);
    assert_eq!(s.adapter.try_mint_and_stake(&short, &short), Err(Ok(AdapterError::MalformedMessage)));
}

#[test]
fn replayed_message_is_refused_by_circle() {
    let env = Env::default();
    let s = setup_stack(&env);
    let me32 = contract_bytes32(&s.adapter.address).unwrap().to_array();
    let sender = Owner::evm(0x11).sender32(&env);
    let amount = (mid_stake(&s) / s.c.scale()) as u64;
    let msg = s.c.inbound_message(&env, domain::ETHEREUM, 6, &s.adapter.address, me32, sender, amount);
    let att = s.c.attest(&env, &msg);
    s.adapter.mint_and_stake(&msg, &att);
    assert!(s.adapter.try_mint_and_stake(&msg, &att).is_err());
}

// --------------------------------------------------------------- outbound --

#[test]
fn withdraw_home_burns_everything_to_the_evm_owner() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);

    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "withdraw_home", ().into_val(&env))]);
    assert_eq!(acct.withdraw_home(), amount);
    assert_eq!(staked(&s, &account), 0);
    assert_eq!(acct.balance(), 0);
    assert_eq!(s.c.usdc.balance(&s.c.tmm.address), 0);
    assert_eq!(owner_to_bytes32(&env, &owner.key(&env)).to_array(), owner.sender32(&env));
}

#[test]
fn forged_owner_signature_cannot_move_funds() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let thief = Owner::evm(0x66);
    let amount = mid_stake(&s);
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    env.set_auths(&[owner_auth(&env, &account, &thief, 1, &account, "withdraw_home", ().into_val(&env))]);
    assert!(acct.try_withdraw_home().is_err());
    assert_eq!(staked(&s, &account), amount);
}

#[test]
fn evm_account_ignores_hook_data_and_pays_its_owner() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let (_, max) = bounds(&s);
    let hook = sol_payout_hook(&env, [0x77; 32], 255);
    let account =
        bridge_with_hook(&env, &s, &s.adapter, owner.domain(), 1, owner.sender32(&env), max * 2, &hook).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.payout_account(), None);
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "send_home", ().into_val(&env))]);
    assert_eq!(acct.send_home(), max * 2);
}

/// security review H-1 (2026-09-24): a Solana payout goes only to the owner's
/// own USDC token account, named by the owner's deposit and saved once. No
/// owner signature at withdrawal can change it.
#[test]
fn solana_payout_account_is_saved_once_from_a_valid_deposit() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::sol(0x22);
    let thief = Owner::sol(0x66);
    let (_, max) = bounds(&s);
    let held = max * 2; // over max: held, not staked
    let unit = s.c.scale();
    let own = sol_payout_hook(&env, owner.sender32(&env), 255);

    // No hook data: the deposit still lands, but nothing can go home yet.
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), held).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.payout_account(), None);
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "send_home", ().into_val(&env))]);
    assert_eq!(acct.try_send_home(), Err(Ok(AccountError::PayoutAccountMissing)));

    // The thief's token account, or garbage, is ignored; the deposit still lands.
    let theirs = sol_payout_hook(&env, thief.sender32(&env), 255);
    bridge_with_hook(&env, &s, &s.adapter, owner.domain(), 2, owner.sender32(&env), unit, &theirs).unwrap().unwrap();
    bridge_with_hook(&env, &s, &s.adapter, owner.domain(), 3, owner.sender32(&env), unit, &[0x33; 33]).unwrap().unwrap();
    assert_eq!(acct.payout_account(), None);
    assert_eq!(acct.balance(), held + 2 * unit);

    // The owner's own token account is saved.
    bridge_with_hook(&env, &s, &s.adapter, owner.domain(), 4, owner.sender32(&env), unit, &own).unwrap().unwrap();
    let saved = BytesN::from_array(&env, &own[..32].try_into().unwrap());
    assert_eq!(acct.payout_account(), Some(saved.clone()));

    // A later deposit naming a different (still derivable) account changes nothing.
    let other_bump = sol_payout_hook(&env, owner.sender32(&env), 254);
    bridge_with_hook(&env, &s, &s.adapter, owner.domain(), 5, owner.sender32(&env), unit, &other_bump).unwrap().unwrap();
    assert_eq!(acct.payout_account(), Some(saved));

    env.set_auths(&[owner_auth(&env, &account, &owner, 2, &account, "send_home", ().into_val(&env))]);
    assert_eq!(acct.send_home(), held + 4 * unit);
    assert_eq!(acct.balance(), 0);
}

#[test]
fn remainder_below_one_canonical_unit_stays_for_next_time() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let (_, max) = bounds(&s);
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), max * 2).unwrap().unwrap();
    let dust = s.c.scale() - 1;
    s.c.usdc_admin.mint(&account, &dust);
    let acct = SafuAccountContractClient::new(&env, &account);
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "send_home", ().into_val(&env))]);
    assert_eq!(acct.send_home(), max * 2);
    assert_eq!(acct.balance(), dust);
}

#[test]
fn nothing_bridgeable_is_an_error() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), mid_stake(&s)).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "send_home", ().into_val(&env))]);
    assert_eq!(acct.try_send_home(), Err(Ok(AccountError::NothingToSend)));
}

// ------------------------------------------------------------------ claim --

/// Stake via CCTP -> oracle-signed claim -> owner approves from home chain ->
/// vesting -> claim_home streams and burns home. The spike's open question
/// ("can a custom account run stake -> approve -> stream?"), answered.
#[test]
fn claim_path_runs_from_a_home_chain_signature_and_pays_home() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);

    env.ledger().with_mut(|li| {
        li.sequence_number += PAST_TIME_GATE;
        li.timestamp += PAST_TIME_GATE as u64 * SECONDS_PER_LEDGER;
    });
    env.mock_all_auths();
    let tx_hash = BytesN::from_array(&env, &[0xcd; 32]);
    let entitlement = amount; // 1x, under every tier ceiling
    let tier = 3u32;
    let hack_ts = env.ledger().timestamp();
    let deadline = hack_ts + 3_600;
    let payload = env.as_contract(&s.pool.address, || {
        protection_pool::testutils::build_approval_payload(
            &env, &account, &tx_hash, entitlement, tier, hack_ts, deadline,
        )
    });
    let bytes: StdVec<u8> = payload.iter().collect();
    let sig = BytesN::from_array(&env, &s.oracle_key.sign(&bytes).to_bytes());
    let claim_id = s.pool.submit_claim(&s.oracle, &account, &tx_hash, &entitlement, &tier, &hack_ts, &deadline, &sig);

    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &s.pool.address, "approve_claim", (claim_id.clone(),).into_val(&env))]);
    s.pool.approve_claim(&claim_id);

    let claim = s.pool.get_claim(&claim_id).unwrap();
    env.ledger().with_mut(|li| {
        li.sequence_number = claim.vesting_ends_ledger + 1;
        li.timestamp += (claim.vesting_ends_ledger - claim.cooldown_ends_ledger) as u64 * SECONDS_PER_LEDGER;
    });
    env.set_auths(&[owner_auth(
        &env, &account, &owner, 2, &account, "claim_home", (claim_id.clone(),).into_val(&env),
    )]);
    let sent = acct.claim_home(&claim_id);
    let streamed = s.pool.get_claim(&claim_id).unwrap().streamed;
    assert!(sent > 0);
    assert_eq!(sent, streamed - streamed % s.c.scale());
    assert!(acct.balance() < s.c.scale());
}

// ---------------------------------------------------------------- backers --

fn backed(s: &Stack, account: &Address) -> (i128, i128) {
    s.pool.get_backer(account).map(|r| (r.amount, r.pending_amount)).unwrap_or((0, 0))
}

#[test]
fn backer_bridge_in_backs_the_pool_from_a_separate_account() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s) * 3; // backers have no stake bounds
    env.set_auths(&[]);
    let account =
        bridge_via(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    assert_eq!(account, s.backer_adapter.account_address(&owner.domain(), &owner.key(&env)));
    assert_ne!(account, s.adapter.account_address(&owner.domain(), &owner.key(&env)));
    assert_eq!(backed(&s, &account), (0, amount)); // pending until maturity
    assert_eq!(staked(&s, &account), 0);
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.mode(), DepositMode::Back);
    assert_eq!(acct.balance(), 0);
    assert_eq!(s.backer_adapter.mode(), DepositMode::Back);
}

#[test]
fn backer_deposit_while_pool_paused_is_held_then_backed_by_owner() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    s.pool.pause();
    let account =
        bridge_via(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    assert_eq!(acct.balance(), amount);
    assert_eq!(backed(&s, &account), (0, 0));
    s.pool.unpause();
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "back_held", (amount,).into_val(&env))]);
    acct.back_held(&amount);
    assert_eq!(backed(&s, &account), (0, amount));
}

/// Back -> maturity -> request -> notice -> complete_back_withdrawal_home:
/// the full amount burns to the owner's home address. Timings come from the
/// pool's own record and setting.
#[test]
fn backer_withdrawal_runs_from_a_home_chain_signature_and_pays_home() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let account =
        bridge_via(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);

    let matures_at = s.pool.get_backer(&account).unwrap().pending_matures_at;
    env.ledger().with_mut(|li| li.timestamp = matures_at + 1);
    s.pool.mature_backing(&account);
    assert_eq!(backed(&s, &account), (amount, 0));

    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "request_back_withdrawal", (amount,).into_val(&env))]);
    let ready_at = acct.request_back_withdrawal(&amount);
    assert_eq!(ready_at, s.pool.get_backer(&account).unwrap().withdraw_ready_at);
    let notice = s.pool.get_setting(&SettingKey::BackerNoticeSeconds) as u64;
    assert!(ready_at >= env.ledger().timestamp() + notice);

    env.ledger().with_mut(|li| li.timestamp = ready_at + 1);
    env.set_auths(&[owner_auth(
        &env, &account, &owner, 2, &account, "complete_back_withdrawal_home", ().into_val(&env),
    )]);
    assert_eq!(acct.complete_back_withdrawal_home(), amount);
    assert_eq!(backed(&s, &account), (0, 0));
    assert_eq!(acct.balance(), 0);
}

#[test]
fn backer_can_cancel_a_pending_withdrawal() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let account =
        bridge_via(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    let matures_at = s.pool.get_backer(&account).unwrap().pending_matures_at;
    env.ledger().with_mut(|li| li.timestamp = matures_at + 1);
    s.pool.mature_backing(&account);
    env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "request_back_withdrawal", (amount,).into_val(&env))]);
    acct.request_back_withdrawal(&amount);
    env.set_auths(&[owner_auth(&env, &account, &owner, 2, &account, "cancel_back_withdrawal", ().into_val(&env))]);
    acct.cancel_back_withdrawal();
    assert_eq!(s.pool.get_backer(&account).unwrap().withdraw_amount, 0);
    assert_eq!(backed(&s, &account), (amount, 0));
}

#[test]
fn forged_signature_cannot_withdraw_backing() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    let account =
        bridge_via(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let acct = SafuAccountContractClient::new(&env, &account);
    let matures_at = s.pool.get_backer(&account).unwrap().pending_matures_at;
    env.ledger().with_mut(|li| li.timestamp = matures_at + 1);
    s.pool.mature_backing(&account);
    let thief = Owner::evm(0x66);
    env.set_auths(&[owner_auth(&env, &account, &thief, 1, &account, "request_back_withdrawal", (amount,).into_val(&env))]);
    assert!(acct.try_request_back_withdrawal(&amount).is_err());
}

// ------------------------------------------------------------------- cost --

/// Live Stellar testnet per-transaction limits, read with
/// `stellar network settings` on 2026-09-22.
const TX_MAX_INSTRUCTIONS: i64 = 400_000_000;
const TX_MEMORY_LIMIT: i64 = 41_943_040;
const TX_MAX_DISK_READ_ENTRIES: u32 = 200;
const TX_MAX_WRITE_ENTRIES: u32 = 200;

/// The heaviest transaction (first deposit: Circle mint + account deploy +
/// stake), all contracts as WASM, must fit the live network limits.
#[test]
fn first_deposit_fits_network_limits() {
    let env = Env::default();
    let s = setup_stack(&env);
    let owner = Owner::evm(0x11);
    let amount = mid_stake(&s);
    env.cost_estimate().disable_resource_limits();
    let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
    let r = env.cost_estimate().resources();
    std::println!(
        "first deposit: instructions={} mem_bytes={} disk_read_entries={} write_entries={}",
        r.instructions, r.mem_bytes, r.disk_read_entries, r.write_entries
    );
    assert_eq!(staked(&s, &account), amount);
    assert!(r.instructions < TX_MAX_INSTRUCTIONS);
    assert!(r.mem_bytes < TX_MEMORY_LIMIT);
    assert!(r.disk_read_entries < TX_MAX_DISK_READ_ENTRIES);
    assert!(r.write_entries < TX_MAX_WRITE_ENTRIES);
}

// ------------------------------------------------------------ yield (r3) --
//
// r3 (2026-09-29): a home-chain owner takes yield without unstaking or
// unbacking, and it burns home. Needs real vault growth, so a minimal vault
// with the DeFindex interface the pool calls (same shape as the pool's own
// test mock, signature verified over RPC) is wired in through governance.

mod yield_home {
    use super::*;
    use protection_pool::{GovChange, GovKind};
    use soroban_sdk::{contract, contractimpl, contracttype};

    #[contracttype]
    enum VKey {
        Token,
        RateBps,
        Shares(Address),
    }

    #[contract]
    struct Vault;

    #[contractimpl]
    impl Vault {
        pub fn init(env: Env, token: Address) {
            env.storage().instance().set(&VKey::Token, &token);
            env.storage().instance().set(&VKey::RateBps, &BPS);
        }
        pub fn set_rate_bps(env: Env, bps: i128) {
            env.storage().instance().set(&VKey::RateBps, &bps);
        }
        pub fn deposit(env: Env, amounts: Vec<i128>, _min: Vec<i128>, from: Address, _invest: bool) -> i128 {
            from.require_auth();
            let amount = amounts.get(0).unwrap();
            let token: Address = env.storage().instance().get(&VKey::Token).unwrap();
            TokenClient::new(&env, &token).transfer(&from, &env.current_contract_address(), &amount);
            let cur: i128 = env.storage().instance().get(&VKey::Shares(from.clone())).unwrap_or(0);
            env.storage().instance().set(&VKey::Shares(from), &(cur + amount));
            amount
        }
        pub fn withdraw(env: Env, shares: i128, min_out: Vec<i128>, from: Address) -> i128 {
            from.require_auth();
            let token: Address = env.storage().instance().get(&VKey::Token).unwrap();
            let rate: i128 = env.storage().instance().get(&VKey::RateBps).unwrap();
            let cur: i128 = env.storage().instance().get(&VKey::Shares(from.clone())).unwrap_or(0);
            env.storage().instance().set(&VKey::Shares(from.clone()), &(cur - shares));
            let out = shares * rate / BPS;
            assert!(out >= min_out.get(0).unwrap_or(0), "below min_amounts_out");
            TokenClient::new(&env, &token).transfer(&env.current_contract_address(), &from, &out);
            out
        }
        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage().instance().get(&VKey::Shares(id)).unwrap_or(0)
        }
        pub fn get_asset_amounts_per_shares(env: Env, vault_shares: i128) -> Vec<i128> {
            let rate: i128 = env.storage().instance().get(&VKey::RateBps).unwrap();
            vec![&env, vault_shares * rate / BPS]
        }
    }

    /// Deploy ceiling for these flows, bps of capacity.
    const DEPLOY_BPS: i128 = 8_000;
    /// Vault gain applied once, bps.
    const GAIN_BPS: i128 = 1_000;

    fn gov(env: &Env, s: &Stack, change: GovChange, kind: GovKind) {
        let (admin, co) = (s.pool.get_admin(), s.pool.get_co_signer());
        s.pool.propose_change(&admin, &change);
        s.pool.approve_change(&co, &change);
        let eta = s.pool.get_pending_change(&kind).unwrap().eta;
        env.ledger().with_mut(|li| li.timestamp = li.timestamp.max(eta));
        s.pool.execute_change(&kind);
    }

    /// Wires a vault, deploys up to the ceiling, then grows it by GAIN_BPS.
    fn deploy_and_grow<'a>(env: &'a Env, s: &Stack<'a>) -> VaultClient<'a> {
        env.mock_all_auths();
        let vault = VaultClient::new(env, &env.register(Vault, ()));
        vault.init(&s.c.usdc.address);
        gov(env, s, GovChange::Vault(vault.address.clone()), GovKind::Vault);
        gov(env, s, GovChange::DeployBps(DEPLOY_BPS), GovKind::DeployBps);
        let amount = s.pool.get_capacity() * DEPLOY_BPS / BPS;
        s.pool.deploy_to_vault(&amount, &0);
        vault.set_rate_bps(&(BPS + GAIN_BPS));
        s.c.usdc_admin.mint(&vault.address, &amount);
        vault
    }

    #[test]
    fn staker_takes_yield_home_and_keeps_the_stake() {
        let env = Env::default();
        let s = setup_stack(&env);
        let owner = Owner::evm(0x11);
        let amount = mid_stake(&s);
        let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
        let acct = SafuAccountContractClient::new(&env, &account);
        deploy_and_grow(&env, &s);
        s.pool.harvest();
        let owed = s.pool.get_staker_yield_owed(&account);
        assert!(owed > s.c.scale(), "test is vacuous without bridgeable yield");

        env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "claim_yield_home", ().into_val(&env))]);
        env.cost_estimate().budget().reset_unlimited();
        env.cost_estimate().disable_resource_limits();
        let sent = acct.claim_yield_home();
        let r = env.cost_estimate().resources();
        std::println!("claim_yield_home: instructions={} mem_bytes={} reads={} writes={}",
            r.instructions, r.mem_bytes, r.disk_read_entries, r.write_entries);
        assert!(r.instructions < TX_MAX_INSTRUCTIONS);
        assert!(r.mem_bytes < TX_MEMORY_LIMIT);
        assert!(r.disk_read_entries < TX_MAX_DISK_READ_ENTRIES);
        assert!(r.write_entries < TX_MAX_WRITE_ENTRIES);

        // Everything bridgeable went home; the sub-unit remainder waits.
        assert_eq!(sent, owed - owed % s.c.scale());
        assert_eq!(acct.balance(), owed % s.c.scale());
        assert_eq!(staked(&s, &account), amount, "stake kept");
        assert_eq!(s.pool.get_staker_yield_owed(&account), 0);
    }

    #[test]
    fn solana_backer_takes_yield_home_and_keeps_the_backing() {
        let env = Env::default();
        let s = setup_stack(&env);
        let owner = Owner::sol(0x22);
        let amount = mid_stake(&s) * 3;
        let payout = sol_payout_hook(&env, owner.sender32(&env), 255);
        let account = bridge_with_hook(&env, &s, &s.backer_adapter, owner.domain(), 1, owner.sender32(&env), amount, &payout)
            .unwrap()
            .unwrap();
        let acct = SafuAccountContractClient::new(&env, &account);
        let matures_at = s.pool.get_backer(&account).unwrap().pending_matures_at;
        env.ledger().with_mut(|li| li.timestamp = matures_at + 1);
        s.pool.mature_backing(&account);
        deploy_and_grow(&env, &s);
        s.pool.harvest();
        let owed = s.pool.get_backer_yield_owed(&account);
        assert!(owed > s.c.scale(), "test is vacuous without bridgeable yield");

        env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "claim_backer_yield_home", ().into_val(&env))]);
        let sent = acct.claim_backer_yield_home();
        assert_eq!(sent, owed - owed % s.c.scale());
        assert_eq!(backed(&s, &account), (amount, 0), "backing kept");
        assert_eq!(s.pool.get_backer_yield_owed(&account), 0);
    }

    #[test]
    fn no_yield_means_nothing_moves() {
        let env = Env::default();
        let s = setup_stack(&env);
        let owner = Owner::evm(0x11);
        let amount = mid_stake(&s);
        let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), amount).unwrap().unwrap();
        let acct = SafuAccountContractClient::new(&env, &account);
        env.set_auths(&[owner_auth(&env, &account, &owner, 1, &account, "claim_yield_home", ().into_val(&env))]);
        assert!(acct.try_claim_yield_home().is_err());
        assert_eq!(staked(&s, &account), amount);
        assert_eq!(acct.balance(), 0);
    }

    #[test]
    fn forged_signature_cannot_take_yield() {
        let env = Env::default();
        let s = setup_stack(&env);
        let owner = Owner::evm(0x11);
        let thief = Owner::evm(0x66);
        let account = bridge_in(&env, &s, owner.domain(), 1, owner.sender32(&env), mid_stake(&s)).unwrap().unwrap();
        let acct = SafuAccountContractClient::new(&env, &account);
        deploy_and_grow(&env, &s);
        s.pool.harvest();
        let owed = s.pool.get_staker_yield_owed(&account);
        env.set_auths(&[owner_auth(&env, &account, &thief, 1, &account, "claim_yield_home", ().into_val(&env))]);
        assert!(acct.try_claim_yield_home().is_err());
        assert_eq!(s.pool.get_staker_yield_owed(&account), owed);
    }
}
