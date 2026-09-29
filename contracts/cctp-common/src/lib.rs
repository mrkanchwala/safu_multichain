// SPDX-License-Identifier: Apache-2.0

//! Shared CCTP v2 definitions for SAFU's Stellar CCTP contracts (v1, Phase 3).
//!
//! Everything the adapter, the per-user account and their tests agree on lives
//! here once: Circle domain ids, the v2 message layout, and how a home-chain
//! burner becomes a SAFU account owner. Circle-side settings that can change
//! (decimals, fees) are NOT copied here; the contracts read them live from
//! Circle's TokenMessengerMinter.
//!
//! Message layout: CCTP v2 `MessageV2` header followed by a `BurnMessageV2`
//! body. Same bytes on every chain.

#![no_std]

use soroban_sdk::{contracttype, Bytes, BytesN, Env};

/// Circle CCTP domain ids.
pub mod domain {
    pub const ETHEREUM: u32 = 0;
    pub const SOLANA: u32 = 5;
    pub const STELLAR: u32 = 27;
}

/// Home chains a SAFU CCTP account can belong to (v1 scope: Ethereum, Solana).
pub const SUPPORTED_HOME_DOMAINS: [u32; 2] = [domain::ETHEREUM, domain::SOLANA];

/// `MessageV2.version` and `BurnMessageV2.version`.
pub const MESSAGE_VERSION: u32 = 1;
pub const BURN_MESSAGE_VERSION: u32 = 1;

/// Standard (finalized) transfer. Stellar outbound is always Standard.
pub const FINALITY_STANDARD: u32 = 2000;

/// Byte offsets inside a `MessageV2` header.
pub mod header {
    pub const VERSION: u32 = 0;
    pub const SOURCE_DOMAIN: u32 = 4;
    pub const DESTINATION_DOMAIN: u32 = 8;
    pub const NONCE: u32 = 12;
    pub const SENDER: u32 = 44;
    pub const RECIPIENT: u32 = 76;
    pub const DESTINATION_CALLER: u32 = 108;
    pub const MIN_FINALITY_THRESHOLD: u32 = 140;
    pub const FINALITY_THRESHOLD_EXECUTED: u32 = 144;
    pub const BODY: u32 = 148;
}

/// Byte offsets inside a `BurnMessageV2` body (relative to the body start).
pub mod burn {
    pub const VERSION: u32 = 0;
    pub const BURN_TOKEN: u32 = 4;
    pub const MINT_RECIPIENT: u32 = 36;
    pub const AMOUNT: u32 = 68;
    pub const MESSAGE_SENDER: u32 = 100;
    pub const MAX_FEE: u32 = 132;
    pub const FEE_EXECUTED: u32 = 164;
    pub const EXPIRATION_BLOCK: u32 = 196;
    pub const HOOK_DATA: u32 = 228;
}

/// Leading zero bytes of an EVM address left-padded to 32 bytes.
pub const EVM_PADDING: u32 = 12;

/// SPL Token program, `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`.
pub const SPL_TOKEN_PROGRAM: [u8; 32] = [
    0x06, 0xdd, 0xf6, 0xe1, 0xd7, 0x65, 0xa1, 0x93,
    0xd9, 0xcb, 0xe1, 0x46, 0xce, 0xeb, 0x79, 0xac,
    0x1c, 0xb4, 0x85, 0xed, 0x5f, 0x5b, 0x37, 0x91,
    0x3a, 0x8c, 0xf5, 0x85, 0x7e, 0xff, 0x00, 0xa9,
];

/// Associated Token Account program, `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL`.
pub const ATA_PROGRAM: [u8; 32] = [
    0x8c, 0x97, 0x25, 0x8f, 0x4e, 0x24, 0x89, 0xf1,
    0xbb, 0x3d, 0x10, 0x29, 0x14, 0x8e, 0x0d, 0x83,
    0x0b, 0x5a, 0x13, 0x99, 0xda, 0xff, 0x10, 0x84,
    0x04, 0x8e, 0x7b, 0xd8, 0xdb, 0xe9, 0xf8, 0x59,
];

/// A Solana deposit's hook data: the owner's USDC token account (32 bytes)
/// then its PDA bump (1 byte), exactly as `findAssociatedTokenPda` returns them.
pub const SOLANA_HOOK_LEN: u32 = 33;

/// Who owns a SAFU CCTP account: the key that burned USDC on the home chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerKey {
    /// 20-byte EVM address (checked by secp256k1 recovery).
    Evm(BytesN<20>),
    /// 32-byte Solana Ed25519 public key.
    Solana(BytesN<32>),
}

/// What an arriving deposit is for. Fixed per adapter at deploy: SAFU runs
/// one adapter per mode, and the user's burn names the adapter, so the intent
/// is chosen by the user's own signed burn and cannot be switched by a relayer.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepositMode {
    /// Stake into the pool (coverage for the staker's wallets).
    Stake,
    /// Back the pool (seed capital, last line of defence).
    Back,
}

/// The fields of an inbound burn message SAFU's adapter acts on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundBurn {
    pub source_domain: u32,
    pub destination_domain: u32,
    pub destination_caller: BytesN<32>,
    pub mint_recipient: BytesN<32>,
    pub message_sender: BytesN<32>,
    /// Home-chain USDC (a Solana mint for Solana burns). Circle's
    /// `receive_message` refuses a token that is not linked to Stellar USDC,
    /// so after a successful receive this is the real USDC mint.
    pub burn_token: BytesN<32>,
    /// Everything after the fixed body. Unchecked by Circle.
    pub hook_data: Bytes,
}

fn read_u32(msg: &Bytes, at: u32) -> Option<u32> {
    let mut out = [0u8; 4];
    for (i, b) in out.iter_mut().enumerate() {
        *b = msg.get(at + i as u32)?;
    }
    Some(u32::from_be_bytes(out))
}

fn read_32(msg: &Bytes, at: u32) -> Option<BytesN<32>> {
    if msg.len() < at + 32 {
        return None;
    }
    msg.slice(at..at + 32).try_into().ok()
}

/// Parses the header and burn body. `None` if the message is too short or
/// either version is not v2. Circle's contracts validate the rest.
pub fn parse_inbound(msg: &Bytes) -> Option<InboundBurn> {
    if msg.len() < header::BODY + burn::HOOK_DATA {
        return None;
    }
    if read_u32(msg, header::VERSION)? != MESSAGE_VERSION {
        return None;
    }
    let body = header::BODY;
    if read_u32(msg, body + burn::VERSION)? != BURN_MESSAGE_VERSION {
        return None;
    }
    Some(InboundBurn {
        source_domain: read_u32(msg, header::SOURCE_DOMAIN)?,
        destination_domain: read_u32(msg, header::DESTINATION_DOMAIN)?,
        destination_caller: read_32(msg, header::DESTINATION_CALLER)?,
        mint_recipient: read_32(msg, body + burn::MINT_RECIPIENT)?,
        message_sender: read_32(msg, body + burn::MESSAGE_SENDER)?,
        burn_token: read_32(msg, body + burn::BURN_TOKEN)?,
        hook_data: msg.slice(body + burn::HOOK_DATA..),
    })
}

/// The Solana owner's USDC token account named in a deposit's hook data, if
/// it really is `owner`'s associated token account for `mint`. Recomputes the
/// PDA (`sha256(owner || token program || mint || bump || ATA program ||
/// "ProgramDerivedAddress")`), so hook data naming anyone else's account, or
/// a typo, gives `None`. Only the owner's own signed burn can carry it.
pub fn solana_payout_account(
    env: &Env,
    owner: &BytesN<32>,
    mint: &BytesN<32>,
    hook_data: &Bytes,
) -> Option<BytesN<32>> {
    if hook_data.len() != SOLANA_HOOK_LEN {
        return None;
    }
    let ata: BytesN<32> = hook_data.slice(0..32).try_into().ok()?;
    let mut pre = Bytes::from_array(env, &owner.to_array());
    pre.extend_from_array(&SPL_TOKEN_PROGRAM);
    pre.extend_from_array(&mint.to_array());
    pre.push_back(hook_data.get(32)?);
    pre.extend_from_array(&ATA_PROGRAM);
    pre.extend_from_slice(b"ProgramDerivedAddress");
    if env.crypto().sha256(&pre).to_array() == ata.to_array() {
        Some(ata)
    } else {
        None
    }
}

/// Maps a burn's (source domain, message sender) to the account owner.
/// `None` for an unsupported domain, or an EVM sender that is not a
/// left-padded 20-byte address.
pub fn owner_from_sender(env: &Env, source_domain: u32, sender: &BytesN<32>) -> Option<OwnerKey> {
    match source_domain {
        domain::ETHEREUM => {
            let raw = sender.to_array();
            if raw[..EVM_PADDING as usize].iter().any(|b| *b != 0) {
                return None;
            }
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&raw[EVM_PADDING as usize..]);
            Some(OwnerKey::Evm(BytesN::from_array(env, &addr)))
        }
        domain::SOLANA => Some(OwnerKey::Solana(sender.clone())),
        _ => None,
    }
}

/// The owner as a CCTP `bytes32` (EVM addresses left-padded).
pub fn owner_to_bytes32(env: &Env, owner: &OwnerKey) -> BytesN<32> {
    match owner {
        OwnerKey::Evm(addr) => {
            let mut out = [0u8; 32];
            out[EVM_PADDING as usize..].copy_from_slice(&addr.to_array());
            BytesN::from_array(env, &out)
        }
        OwnerKey::Solana(key) => key.clone(),
    }
}

/// Deterministic deploy salt of an owner's account:
/// `sha256(home_domain_be || owner_bytes32)`.
pub fn account_salt(env: &Env, home_domain: u32, owner: &OwnerKey) -> BytesN<32> {
    let mut pre = Bytes::from_array(env, &home_domain.to_be_bytes());
    pre.append(&owner_to_bytes32(env, owner).into());
    env.crypto().sha256(&pre).to_bytes()
}

/// XDR prefix of a contract address: `ScVal::Address` (18), then
/// `ScAddress::Contract` (1), then the 32-byte contract id.
const CONTRACT_ADDRESS_XDR_PREFIX: [u8; 8] = [0, 0, 0, 18, 0, 0, 0, 1];

/// A Soroban contract address as a CCTP `bytes32` (its contract id hash).
/// `None` for an account (G...) address. Read from the XDR encoding so no
/// `hazmat` SDK feature is needed (features unify across the workspace and
/// could change the pool's WASM).
pub fn contract_bytes32(addr: &soroban_sdk::Address) -> Option<BytesN<32>> {
    use soroban_sdk::xdr::ToXdr;
    let xdr = addr.clone().to_xdr(addr.env());
    let prefix = CONTRACT_ADDRESS_XDR_PREFIX.len() as u32;
    if xdr.len() != prefix + 32 {
        return None;
    }
    for (i, b) in CONTRACT_ADDRESS_XDR_PREFIX.iter().enumerate() {
        if xdr.get(i as u32)? != *b {
            return None;
        }
    }
    xdr.slice(prefix..).try_into().ok()
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    fn message(env: &Env, version: u32, body_version: u32, len: u32) -> Bytes {
        let mut m = Bytes::new(env);
        m.extend_from_array(&version.to_be_bytes());
        m.extend_from_array(&domain::SOLANA.to_be_bytes());
        m.extend_from_array(&domain::STELLAR.to_be_bytes());
        while m.len() < header::BODY {
            m.push_back(0);
        }
        m.extend_from_array(&body_version.to_be_bytes());
        while m.len() < len {
            m.push_back(0xee);
        }
        m
    }

    #[test]
    fn parses_a_minimal_v2_burn() {
        let env = Env::default();
        let b = parse_inbound(&message(&env, MESSAGE_VERSION, BURN_MESSAGE_VERSION, header::BODY + burn::HOOK_DATA)).unwrap();
        assert_eq!(b.source_domain, domain::SOLANA);
        assert_eq!(b.destination_domain, domain::STELLAR);
        assert_eq!(b.message_sender.to_array(), [0xee; 32]);
    }

    #[test]
    fn rejects_short_messages_and_other_versions() {
        let env = Env::default();
        let full = header::BODY + burn::HOOK_DATA;
        assert!(parse_inbound(&message(&env, MESSAGE_VERSION, BURN_MESSAGE_VERSION, full - 1)).is_none());
        assert!(parse_inbound(&message(&env, MESSAGE_VERSION + 1, BURN_MESSAGE_VERSION, full)).is_none());
        assert!(parse_inbound(&message(&env, MESSAGE_VERSION, BURN_MESSAGE_VERSION + 1, full)).is_none());
    }

    #[test]
    fn evm_sender_round_trips_and_padding_is_enforced() {
        let env = Env::default();
        let mut raw = [0u8; 32];
        raw[EVM_PADDING as usize..].copy_from_slice(&[0x11; 20]);
        let sender = BytesN::from_array(&env, &raw);
        let owner = owner_from_sender(&env, domain::ETHEREUM, &sender).unwrap();
        assert_eq!(owner, OwnerKey::Evm(BytesN::from_array(&env, &[0x11; 20])));
        assert_eq!(owner_to_bytes32(&env, &owner), sender);
        raw[0] = 1;
        assert!(owner_from_sender(&env, domain::ETHEREUM, &BytesN::from_array(&env, &raw)).is_none());
    }

    #[test]
    fn only_supported_domains_map_to_an_owner() {
        let env = Env::default();
        let s = BytesN::from_array(&env, &[0u8; 32]);
        for d in SUPPORTED_HOME_DOMAINS {
            assert!(owner_from_sender(&env, d, &s).is_some());
        }
        assert!(owner_from_sender(&env, domain::STELLAR, &s).is_none());
    }

    #[test]
    fn salt_differs_by_domain_and_owner() {
        let env = Env::default();
        let a = OwnerKey::Solana(BytesN::from_array(&env, &[1; 32]));
        let b = OwnerKey::Solana(BytesN::from_array(&env, &[2; 32]));
        assert_ne!(account_salt(&env, domain::SOLANA, &a), account_salt(&env, domain::SOLANA, &b));
        assert_ne!(account_salt(&env, domain::SOLANA, &a), account_salt(&env, domain::ETHEREUM, &a));
    }

    fn b32(env: &Env, hex: &str) -> BytesN<32> {
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        BytesN::from_array(env, &out)
    }

    fn hook(env: &Env, ata: &BytesN<32>, bump: u8) -> Bytes {
        let mut h = Bytes::from_array(env, &ata.to_array());
        h.push_back(bump);
        h
    }

    /// Vectors from `@solana/web3.js` `PublicKey.findProgramAddressSync`
    /// (mainnet and devnet USDC; the second has a non-255 bump).
    #[test]
    fn solana_payout_account_matches_web3js_and_rejects_anything_else() {
        let env = Env::default();
        let vectors = [
            (
                "6752055c20b3e9d8746656ddf73855507f87ab6d87523e4c76a7fa36096a99eb",
                "c6fa7af3bedbad3a3d65f36aabc97431b1bbe4c2d2f6e0e47ca60203452f5d61",
                "a44ea6f2bacfbf0778851373b51a8e87b4261458d7fd93871c9cda790f0e1b26",
                255u8,
            ),
            (
                "7e8c088760bfde1dddcf32c17f209b8242ee52aaf131facd88d0ea2c6d0b06f2",
                "3b442cb3912157f13a933d0134282d032b5ffecd01a2dbf1b7790608df002ea7",
                "fbc5b385330af825045a1ae2406b2dcc98ca4785011212725ad7a067977ac16f",
                253u8,
            ),
        ];
        for (owner, mint, ata, bump) in vectors {
            let (o, m, a) = (b32(&env, owner), b32(&env, mint), b32(&env, ata));
            assert_eq!(solana_payout_account(&env, &o, &m, &hook(&env, &a, bump)), Some(a.clone()));
            // Wrong bump, other owner, other mint, someone else's account, bad length.
            assert!(solana_payout_account(&env, &o, &m, &hook(&env, &a, bump - 1)).is_none());
            assert!(solana_payout_account(&env, &m, &m, &hook(&env, &a, bump)).is_none());
            assert!(solana_payout_account(&env, &o, &o, &hook(&env, &a, bump)).is_none());
            assert!(solana_payout_account(&env, &o, &m, &hook(&env, &o, bump)).is_none());
            assert!(solana_payout_account(&env, &o, &m, &Bytes::from_array(&env, &a.to_array())).is_none());
            let mut long = hook(&env, &a, bump);
            long.push_back(0);
            assert!(solana_payout_account(&env, &o, &m, &long).is_none());
        }
    }

    #[test]
    fn parse_reads_burn_token_and_hook_data() {
        let env = Env::default();
        let mut m = message(&env, MESSAGE_VERSION, BURN_MESSAGE_VERSION, header::BODY + burn::HOOK_DATA);
        let b = parse_inbound(&m).unwrap();
        assert_eq!(b.burn_token.to_array(), [0xee; 32]);
        assert_eq!(b.hook_data.len(), 0);
        m.extend_from_array(&[7u8; 33]);
        assert_eq!(parse_inbound(&m).unwrap().hook_data, Bytes::from_array(&env, &[7u8; 33]));
    }

    #[test]
    fn contract_bytes32_only_for_contract_addresses() {
        let env = Env::default();
        let contract = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
        assert!(contract_bytes32(&contract).is_some());
        let account = Address::from_str(&env, "GC3UMF22VAUKANSGWTVZX722ZHMAXPFVDWRKO7ETLEXMCEXVDGSCVUG2");
        assert!(contract_bytes32(&account).is_none());
    }
}
