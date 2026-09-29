//! Solana devnet sender for the SAFU covered-wallet registry, over LayerZero.
//!
//! Hand-written, no Anchor and no LayerZero crates: it builds the 65-byte message
//! `staker_hash(32) ++ chain_id(1) ++ wallet_hash(32)` (chain_id 2 = Solana) and forwards a `send` to
//! LayerZero's real endpoint program, signing as the `Store` PDA (the OApp address).
//!
//! Testnet demo. One admin, no receive path, destination fixed to Stellar testnet (eid 40600).
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

entrypoint!(process);

const STORE_SEED: &[u8] = b"Store";
const DST_EID: u32 = 40600;
const CHAIN_SOLANA: u8 = 2;
const MESSAGE_LEN: usize = 65;
/// Anchor discriminators of the LayerZero endpoint instructions: sha256("global:<name>")[..8].
const DISC_REGISTER_OAPP: [u8; 8] = [129, 89, 71, 68, 11, 82, 210, 125];
const DISC_SEND: [u8; 8] = [102, 251, 20, 187, 65, 75, 12, 69];

/// Account layout for `Register` (index -> role). 0..=2 are ours, 3..=26 are forwarded verbatim.
/// 3 ULN program | 4 sender send-lib cfg | 5 default send-lib cfg | 6 send-lib info | 7 endpoint settings
/// 8 nonce | 9 endpoint event authority | 10 endpoint program | then the message library's accounts.
const FORWARD_FROM: usize = 3;
const ENDPOINT_FIXED: usize = 8; // accounts 3..=10, after the sender

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    match data.first() {
        Some(0) => init_store(program_id, accounts, &data[1..]),
        Some(1) => register(program_id, accounts, &data[1..]),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// data: admin(32). accounts: payer(S,W), store(W), system, endpoint program, oapp_registry(W), endpoint event authority.
fn init_store(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if data.len() != 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let it = &mut accounts.iter();
    let payer = next_account_info(it)?;
    let store = next_account_info(it)?;
    let system = next_account_info(it)?;
    let endpoint = next_account_info(it)?;
    let registry = next_account_info(it)?;
    let event_auth = next_account_info(it)?;

    let (expected, bump) = Pubkey::find_program_address(&[STORE_SEED], program_id);
    if expected != *store.key || !payer.is_signer {
        return Err(ProgramError::InvalidSeeds);
    }
    let seeds: &[&[u8]] = &[STORE_SEED, &[bump]];
    if store.data_is_empty() {
        let space = 1 + 32;
        let lamports = Rent::get()?.minimum_balance(space);
        invoke_signed(
            &system_instruction::create_account(payer.key, store.key, lamports, space as u64, program_id),
            &[payer.clone(), store.clone(), system.clone()],
            &[seeds],
        )?;
        let mut d = store.try_borrow_mut_data()?;
        d[0] = bump;
        d[1..33].copy_from_slice(data);
    }

    // register_oapp(delegate = admin)
    let mut ix_data = DISC_REGISTER_OAPP.to_vec();
    ix_data.extend_from_slice(data);
    let ix = Instruction {
        program_id: *endpoint.key,
        accounts: vec![
            AccountMeta::new(*payer.key, true),
            AccountMeta::new_readonly(*store.key, true),
            AccountMeta::new(*registry.key, false),
            AccountMeta::new_readonly(*system.key, false),
            AccountMeta::new_readonly(*event_auth.key, false),
            AccountMeta::new_readonly(*endpoint.key, false),
        ],
        data: ix_data,
    };
    invoke_signed(
        &ix,
        &[payer.clone(), store.clone(), registry.clone(), system.clone(), event_auth.clone(), endpoint.clone()],
        &[seeds],
    )
}

/// data: staker_hash(32) wallet_hash(32) native_fee(u64) receiver(32) options_len(u16) options.
/// accounts: payer(S,W), store, endpoint program, then 24 forwarded accounts (see layout above).
fn register(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if data.len() < 32 + 32 + 8 + 32 + 2 || accounts.len() < FORWARD_FROM + 24 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let staker_hash = &data[0..32];
    let wallet_hash = &data[32..64];
    let native_fee = u64::from_le_bytes(data[64..72].try_into().unwrap());
    let receiver: [u8; 32] = data[72..104].try_into().unwrap();
    let olen = u16::from_le_bytes(data[104..106].try_into().unwrap()) as usize;
    if data.len() != 106 + olen {
        return Err(ProgramError::InvalidInstructionData);
    }
    let options = &data[106..];

    let payer = &accounts[0];
    let store = &accounts[1];
    let endpoint = &accounts[2];
    let (expected, bump) = Pubkey::find_program_address(&[STORE_SEED], program_id);
    if expected != *store.key || !payer.is_signer {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut message = Vec::with_capacity(MESSAGE_LEN);
    message.extend_from_slice(staker_hash);
    message.push(CHAIN_SOLANA);
    message.extend_from_slice(wallet_hash);

    // SendParams { dst_eid u32, receiver [u8;32], message Vec<u8>, options Vec<u8>, native_fee u64, lz_token_fee u64 }
    let mut ix_data = DISC_SEND.to_vec();
    ix_data.extend_from_slice(&DST_EID.to_le_bytes());
    ix_data.extend_from_slice(&receiver);
    ix_data.extend_from_slice(&(message.len() as u32).to_le_bytes());
    ix_data.extend_from_slice(&message);
    ix_data.extend_from_slice(&(options.len() as u32).to_le_bytes());
    ix_data.extend_from_slice(options);
    ix_data.extend_from_slice(&native_fee.to_le_bytes());
    ix_data.extend_from_slice(&0u64.to_le_bytes());

    // Endpoint `send` accounts: the sender (this PDA, signing), then the forwarded accounts in order.
    let forwarded = &accounts[FORWARD_FROM..];
    let mut metas = vec![AccountMeta::new_readonly(*store.key, true)];
    for a in forwarded {
        metas.push(if a.is_writable { AccountMeta::new(*a.key, a.is_signer) } else { AccountMeta::new_readonly(*a.key, a.is_signer) });
    }
    let ix = Instruction { program_id: *endpoint.key, accounts: metas, data: ix_data };
    let mut infos: Vec<AccountInfo> = vec![store.clone()];
    infos.extend(forwarded.iter().cloned());
    infos.push(endpoint.clone());
    let _ = ENDPOINT_FIXED;
    invoke_signed(&ix, &infos, &[&[STORE_SEED, &[bump]]])
}
