//! v1 (2026-09-23, pre-audit hardening): fuzz target for the CCTP path, on the
//! REAL stack: Circle's CCTP v2 contracts, the SAFU pool, both adapters (stake
//! and back) and the per-user `safu-account` they deploy, all as release WASM.
//! Owner actions carry real signed Soroban auth entries (EVM personal_sign /
//! Solana Ed25519), never mocked auth.
//!
//! Ops: bridge-ins (clean, or tampered: wrong caller, wrong recipient, other
//! source domain, bad EVM padding, truncated, flipped byte), replays, every
//! owner action (signed by the owner or by a thief), backer maturity, pause,
//! time travel.
//!
//! Invariants, checked after EVERY op:
//!   1. Conservation: sum over accounts of (held USDC + live stake + backing)
//!      + everything burned home == everything minted in. Nothing is lost,
//!      nothing is created.
//!   2. The adapters never keep USDC; Circle's messenger never keeps USDC; no
//!      account leaves a standing allowance to the messenger.
//!   3. The pool's USDC balance == total staked + total backed + pending backing.
//!   4. A clean, attested burn with a non-zero amount ALWAYS lands (a CCTP mint
//!      cannot be reversed, so SAFU must never strand one), in the account
//!      derived from (source domain, burner), whose mode matches the adapter.
//!   5. Each tamper is rejected with its exact error; any rejection moves no
//!      money and leaves the Circle nonce unused. A replay never lands.
//!   6. A thief's signature never succeeds and moves nothing. A failed owner
//!      action moves nothing. A successful send-home burns exactly what left
//!      the account, in whole canonical units. A Solana send-home only
//!      succeeds once a payout account is saved.
//!   7. (security review H-1, 2026-09-24) A Solana account's saved payout account is
//!      exactly the first valid one any landed deposit carried: the owner's
//!      own associated token account for the burned mint, never changed after,
//!      never set from an EVM deposit, a thief's account or garbage.
//!
//! Run (after `stellar contract build` in contracts/):
//! `cargo +nightly fuzz run fuzz_cctp_flows -- -max_total_time=600`

#![no_main]

mod harness;

use arbitrary::Arbitrary;
use cctp_common::{
    burn, contract_bytes32, domain, header, owner_from_sender, DepositMode, ATA_PROGRAM, SOLANA_HOOK_LEN,
    SPL_TOKEN_PROGRAM, SUPPORTED_HOME_DOMAINS,
};
use harness::{owner_auth, setup_circle, sol_payout_hook, Owner};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, BytesN, Env, IntoVal, Val, Vec};

use cctp_adapter::{AdapterError, CctpAdapterClient};
use protection_pool::{ProtectionPoolClient, SettingKey};
use safu_account::SafuAccountContractClient;

const ACCOUNT_WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/safu_account.wasm");
const ADAPTER_WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/cctp_adapter.wasm");
const POOL_WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/protection_pool.wasm");

const POOL_CAP: i128 = 1_000_000_000_000;
const BPS: i128 = 10_000;
const MAX_OPS: usize = 32;
const MAX_TOTAL_DAYS: u32 = 365;
const NUM_OWNERS: usize = 4;

#[derive(Arbitrary, Debug)]
enum Tamper {
    None,
    WrongCaller,
    WrongRecipient,
    SourceDomain(u32),
    BadEvmPadding(u8),
    Truncate(u16),
    Flip { at: u16, xor: u8 },
}

#[derive(Arbitrary, Debug)]
enum Action {
    SendHome,
    WithdrawHome,
    ExitHome,
    StakeHeld { amount: i64 },
    BackHeld { amount: i64 },
    RequestBackWithdrawal { amount: i64 },
    CancelBackWithdrawal,
    CompleteBackWithdrawalHome,
}

/// Hook data on a burn.
#[derive(Arbitrary, Debug)]
enum Hook {
    None,
    /// The burner's own token account (any bump derives; 255 is canonical).
    Own { bump: u8 },
    /// Another owner's (or a thief's) token account.
    Other { who: u8, bump: u8 },
    Raw(std::vec::Vec<u8>),
}

#[derive(Arbitrary, Debug)]
enum Op {
    Bridge { owner: u8, back: bool, units: u32, tamper: Tamper, hook: Hook },
    Replay { idx: u8 },
    Act { owner: u8, back: bool, action: Action, thief: bool },
    Mature { owner: u8, back: bool },
    AdvanceDays { days: u8 },
    Pause,
    Unpause,
}

fn ok<T, E1, E2>(r: Result<Result<T, E1>, E2>) -> Option<T> {
    match r {
        Ok(Ok(v)) => Some(v),
        _ => None,
    }
}

fn read_32(msg: &[u8], at: u32) -> Option<[u8; 32]> {
    msg.get(at as usize..at as usize + 32).map(|s| s.try_into().unwrap())
}

fn read_u32(msg: &[u8], at: u32) -> Option<u32> {
    msg.get(at as usize..at as usize + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}

/// Model of the payout account a landed message should offer: Solana source,
/// exactly 33 bytes of hook data, and the PDA of (sender, burn token, bump).
fn model_payout(env: &Env, msg: &[u8]) -> Option<[u8; 32]> {
    if read_u32(msg, header::SOURCE_DOMAIN)? != domain::SOLANA {
        return None;
    }
    let hook = msg.get((header::BODY + burn::HOOK_DATA) as usize..)?;
    if hook.len() != SOLANA_HOOK_LEN as usize {
        return None;
    }
    let mut pre = std::vec::Vec::new();
    pre.extend_from_slice(&read_32(msg, header::BODY + burn::MESSAGE_SENDER)?);
    pre.extend_from_slice(&SPL_TOKEN_PROGRAM);
    pre.extend_from_slice(&read_32(msg, header::BODY + burn::BURN_TOKEN)?);
    pre.push(hook[32]);
    pre.extend_from_slice(&ATA_PROGRAM);
    pre.extend_from_slice(b"ProgramDerivedAddress");
    let pda = env.crypto().sha256(&Bytes::from_slice(env, &pre)).to_array();
    (pda[..] == hook[..32]).then_some(pda)
}

fuzz_target!(|ops: std::vec::Vec<Op>| {
    let env = Env::default();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
        li.min_persistent_entry_ttl = 10_000_000;
        li.min_temp_entry_ttl = 10_000_000;
        li.max_entry_ttl = 20_000_000;
    });
    let c = setup_circle(&env);
    let admin = Address::generate(&env);
    let pool_id = env.register(
        POOL_WASM,
        (
            admin,
            Address::generate(&env),
            BytesN::from_array(&env, &[7u8; 32]), // no claims in this target
            Address::generate(&env),
            Address::generate(&env), // guardian
            c.usdc.address.clone(),
            POOL_CAP,
        ),
    );
    let pool = ProtectionPoolClient::new(&env, &pool_id);
    let wasm = env.deployer().upload_contract_wasm(ACCOUNT_WASM);
    let adapter_args = |mode: DepositMode| {
        (c.mt.address.clone(), c.tmm.address.clone(), c.usdc.address.clone(), pool_id.clone(), wasm.clone(), mode)
    };
    let adapters = [
        CctpAdapterClient::new(&env, &env.register(ADAPTER_WASM, adapter_args(DepositMode::Stake))),
        CctpAdapterClient::new(&env, &env.register(ADAPTER_WASM, adapter_args(DepositMode::Back))),
    ];
    let scale = c.scale();
    let max_stake = POOL_CAP * pool.get_setting(&SettingKey::MaxStakeBps) / BPS;
    let max_units = (max_stake / scale) as u32;
    env.cost_estimate().budget().reset_unlimited();

    let owners = [Owner::evm(0x11), Owner::evm(0x12), Owner::sol(0x22), Owner::sol(0x23)];
    let thieves = [Owner::evm(0x66), Owner::sol(0x77)];

    // Every account that has ever received money.
    let mut accounts: std::vec::Vec<Address> = std::vec::Vec::new();
    // Accepted (adapter index, message, attestation), for replays.
    let mut accepted: std::vec::Vec<(usize, Bytes, Bytes)> = std::vec::Vec::new();
    // Model of each account's saved payout account (invariant 7).
    // Address has no Hash impl, so a Vec with linear lookup (a handful of accounts per run).
    let mut payouts: std::vec::Vec<(Address, [u8; 32])> = std::vec::Vec::new();
    let mut minted_in = 0i128;
    let mut burned_home = 0i128;
    let mut next_nonce = 0u32;
    let mut next_auth = 0i64;
    let mut days_used = 0u32;

    let holdings = |a: &Address| -> i128 {
        let staked = pool.get_stake(a).map(|r| if r.withdrawn { 0 } else { r.amount }).unwrap_or(0);
        let backed = pool.get_backer(a).map(|r| r.amount + r.pending_amount).unwrap_or(0);
        c.usdc.balance(a) + staked + backed
    };
    let snapshot = |accts: &[Address]| -> std::vec::Vec<i128> { accts.iter().map(|a| holdings(a)).collect() };

    for op in ops.into_iter().take(MAX_OPS) {
        match op {
            Op::Bridge { owner, back, units, tamper, hook } => {
                let o = &owners[owner as usize % NUM_OWNERS];
                let ai = back as usize;
                let adapter = &adapters[ai];
                let canonical = (units % (3 * max_units + 2)) as u64; // 0, in bounds, over max
                next_nonce += 1;
                let mut nonce = [0u8; 32];
                nonce[28..].copy_from_slice(&next_nonce.to_be_bytes());

                let me32 = contract_bytes32(&adapter.address).unwrap().to_array();
                let mut source = o.domain();
                let mut sender = o.sender32(&env);
                let mut caller = me32;
                let mut recipient = adapter.address.clone();
                let mut expect: Option<AdapterError> = None;
                match tamper {
                    Tamper::WrongCaller => {
                        caller = [0xab; 32];
                        expect = Some(AdapterError::WrongDestinationCaller);
                    }
                    Tamper::WrongRecipient => {
                        recipient = adapters[1 - ai].address.clone();
                        expect = Some(AdapterError::WrongMintRecipient);
                    }
                    Tamper::SourceDomain(d) if !SUPPORTED_HOME_DOMAINS.contains(&d) => {
                        source = d;
                        expect = Some(AdapterError::UnsupportedDomain);
                    }
                    Tamper::BadEvmPadding(b) if o.is_evm() => {
                        sender[0] = b | 1;
                        expect = Some(AdapterError::UnsupportedSender);
                    }
                    _ => {}
                }
                let mut msg = c.inbound_message(source, nonce, &recipient, caller, sender, canonical);
                msg.extend_from_slice(&match hook {
                    Hook::None => std::vec::Vec::new(),
                    Hook::Own { bump } => sol_payout_hook(&env, o.sender32(&env), bump),
                    Hook::Other { who, bump } => {
                        let i = who as usize % (owners.len() + thieves.len());
                        let other = if i < owners.len() { &owners[i] } else { &thieves[i - owners.len()] };
                        sol_payout_hook(&env, other.sender32(&env), bump)
                    }
                    Hook::Raw(mut b) => {
                        b.truncate(64);
                        b
                    }
                });
                let clean = matches!(tamper, Tamper::None);
                match tamper {
                    Tamper::Truncate(n) => {
                        // Modulo the v2 minimum, not msg.len(): cutting only the hook
                        // leaves a valid message, and bad hook data never blocks a deposit.
                        msg.truncate(n as usize % (header::BODY + burn::HOOK_DATA) as usize);
                        expect = Some(AdapterError::MalformedMessage);
                    }
                    Tamper::Flip { at, xor } => {
                        let len = msg.len();
                        msg[at as usize % len] ^= xor;
                    }
                    _ => {}
                }
                let msg_b = Bytes::from_slice(&env, &msg);
                let att = c.attest(&env, &msg_b);

                env.set_auths(&[]); // permissionless, nothing mocked
                let before = snapshot(&accounts);
                let got = adapter.try_mint_and_stake(&msg_b, &att);

                if let Some(e) = expect {
                    assert_eq!(got, Err(Ok(e)), "tamper not rejected with its own error");
                }
                let got_dbg = format!("{got:?}");
                match ok(got) {
                    Some(account) => {
                        // Where the money landed must follow from the message itself.
                        let src = read_u32(&msg, header::SOURCE_DOMAIN).unwrap();
                        let snd = read_32(&msg, header::BODY + burn::MESSAGE_SENDER).unwrap();
                        let key = owner_from_sender(&env, src, &BytesN::from_array(&env, &snd))
                            .expect("LANDED FROM AN UNSUPPORTED BURNER");
                        assert_eq!(account, adapter.account_address(&src, &key), "money landed in the wrong account");
                        let acct = SafuAccountContractClient::new(&env, &account);
                        assert_eq!(acct.mode(), adapter.mode(), "account mode differs from its adapter");
                        assert_eq!(acct.adapter(), adapter.address, "account names another adapter");

                        if let Some(p) = model_payout(&env, &msg) {
                            if !payouts.iter().any(|(a, _)| a == &account) {
                                payouts.push((account.clone(), p));
                            }
                        }
                        assert_eq!(
                            acct.payout_account().map(|b| b.to_array()),
                            payouts.iter().find(|(a, _)| a == &account).map(|(_, p)| *p),
                            "PAYOUT ACCOUNT differs from the first valid one deposited"
                        );

                        if !accounts.contains(&account) {
                            accounts.push(account.clone());
                        }
                        let after = snapshot(&accounts);
                        let mut gained = 0i128;
                        for (i, a) in accounts.iter().enumerate() {
                            let was = before.get(i).copied().unwrap_or(0);
                            if *a == account {
                                gained = after[i] - was;
                            } else {
                                assert_eq!(after[i], was, "a bridge-in moved another account's money");
                            }
                        }
                        assert!(gained > 0, "bridge-in reported success but credited nothing");
                        if clean {
                            assert_eq!(gained, canonical as i128 * scale, "clean bridge-in credited the wrong amount");
                        }
                        minted_in += gained;
                        accepted.push((ai, msg_b, att));
                    }
                    None => {
                        assert!(!(clean && canonical > 0), "CLEAN ATTESTED BURN STRANDED: {got_dbg}");
                        assert_eq!(snapshot(&accounts), before, "a rejected bridge-in moved money");
                        if let Some(n) = read_32(&msg, header::NONCE) {
                            assert!(!c.mt.is_nonce_used(&BytesN::from_array(&env, &n)), "rejected message consumed");
                        }
                    }
                }
            }
            Op::Replay { idx } => {
                if !accepted.is_empty() {
                    let (ai, msg, att) = &accepted[idx as usize % accepted.len()];
                    env.set_auths(&[]);
                    let before = snapshot(&accounts);
                    assert!(adapters[*ai].try_mint_and_stake(msg, att).is_err(), "REPLAY LANDED");
                    assert_eq!(snapshot(&accounts), before, "a replay moved money");
                }
            }
            Op::Act { owner, back, action, thief } => {
                let oi = owner as usize % NUM_OWNERS;
                let o = &owners[oi];
                let account = adapters[back as usize].account_address(&o.domain(), &o.key(&env));
                if !accounts.contains(&account) {
                    continue; // never deployed
                }
                let signer = if thief { &thieves[if o.is_evm() { 0 } else { 1 }] } else { o };
                let none = || Vec::<Val>::new(&env);
                let (name, args, sends): (&str, Vec<Val>, bool) = match &action {
                    Action::SendHome => ("send_home", none(), true),
                    Action::WithdrawHome => ("withdraw_home", none(), true),
                    Action::ExitHome => ("exit_home", none(), true),
                    Action::StakeHeld { amount } => ("stake_held", (*amount as i128,).into_val(&env), false),
                    Action::BackHeld { amount } => ("back_held", (*amount as i128,).into_val(&env), false),
                    Action::RequestBackWithdrawal { amount } => {
                        ("request_back_withdrawal", (*amount as i128,).into_val(&env), false)
                    }
                    Action::CancelBackWithdrawal => ("cancel_back_withdrawal", Vec::<Val>::new(&env), false),
                    Action::CompleteBackWithdrawalHome => ("complete_back_withdrawal_home", none(), true),
                };
                next_auth += 1;
                env.set_auths(&[owner_auth(&env, &account, signer, next_auth, &account, name, args)]);
                let acct = SafuAccountContractClient::new(&env, &account);
                let before = snapshot(&accounts);
                let res: Option<i128> = match action {
                    Action::SendHome => ok(acct.try_send_home()),
                    Action::WithdrawHome => ok(acct.try_withdraw_home()),
                    Action::ExitHome => ok(acct.try_exit_home()),
                    Action::StakeHeld { amount } => ok(acct.try_stake_held(&(amount as i128))).map(|_| 0),
                    Action::BackHeld { amount } => ok(acct.try_back_held(&(amount as i128))).map(|_| 0),
                    Action::RequestBackWithdrawal { amount } => {
                        ok(acct.try_request_back_withdrawal(&(amount as i128))).map(|_| 0)
                    }
                    Action::CancelBackWithdrawal => ok(acct.try_cancel_back_withdrawal()).map(|_| 0),
                    Action::CompleteBackWithdrawalHome => ok(acct.try_complete_back_withdrawal_home()),
                };
                let after = snapshot(&accounts);
                match res {
                    None => assert_eq!(after, before, "a failed owner action moved money"),
                    Some(_) if thief => panic!("THIEF SIGNATURE SUCCEEDED: {name}"),
                    Some(sent) => {
                        for (i, a) in accounts.iter().enumerate() {
                            let expected = if *a == account && sends { before[i] - sent } else { before[i] };
                            assert_eq!(after[i], expected, "{name}: money moved other than the burn home");
                        }
                        if sends {
                            assert!(sent > 0 && sent % scale == 0, "{name}: burn of {sent} is not whole canonical units");
                            if !o.is_evm() {
                                assert!(payouts.iter().any(|(a, _)| a == &account), "Solana burn home with no saved payout account");
                            }
                            burned_home += sent;
                        }
                    }
                }
            }
            Op::Mature { owner, back } => {
                let o = &owners[owner as usize % NUM_OWNERS];
                let account = adapters[back as usize].account_address(&o.domain(), &o.key(&env));
                env.set_auths(&[]); // permissionless
                let before = snapshot(&accounts);
                let _ = pool.try_mature_backing(&account);
                assert_eq!(snapshot(&accounts), before, "maturing moved money");
            }
            Op::AdvanceDays { days } => {
                let d = 1 + (days as u32 % 30);
                if days_used + d <= MAX_TOTAL_DAYS {
                    days_used += d;
                    env.ledger().with_mut(|li| {
                        li.sequence_number += d * 17_280;
                        li.timestamp += d as u64 * 86_400;
                    });
                }
            }
            Op::Pause => {
                env.mock_all_auths();
                let _ = pool.try_pause();
            }
            Op::Unpause => {
                env.mock_all_auths();
                let _ = pool.try_unpause();
            }
        }

        // Global invariants after every op.
        let held: i128 = accounts.iter().map(|a| holdings(a)).sum();
        assert_eq!(held + burned_home, minted_in, "CONSERVATION VIOLATED");
        for a in &adapters {
            assert_eq!(c.usdc.balance(&a.address), 0, "adapter kept USDC");
        }
        assert_eq!(c.usdc.balance(&c.tmm.address), 0, "messenger kept USDC");
        for a in &accounts {
            assert_eq!(c.usdc.allowance(a, &c.tmm.address), 0, "standing allowance to the messenger");
        }
        assert_eq!(
            c.usdc.balance(&pool_id),
            pool.get_total_staked() + pool.get_total_backed() + pool.get_total_backed_pending(),
            "pool balance != its records"
        );
    }
});
