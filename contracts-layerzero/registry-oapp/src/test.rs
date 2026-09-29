extern crate std;

use crate::{RegistryOApp, RegistryOAppClient, CHAIN_ETH, CHAIN_SOLANA, MESSAGE_LEN};
use endpoint_v2::Origin;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{storage::Persistent as _, Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    Address, Bytes, BytesN, Env, IntoVal,
};

const ETH_EID: u32 = 40161;
const SOL_EID: u32 = 40168;

// Minimal endpoint: only what `lz_receive` and the constructor touch.
#[contract]
pub struct MockEndpoint;

#[contractimpl]
impl MockEndpoint {
    pub fn set_delegate(_env: Env, _oapp: &Address, _delegate: &Option<Address>) {}
    pub fn native_token(env: Env) -> Address {
        Address::generate(&env)
    }
    pub fn clear(_env: Env, _caller: Address, _origin: Origin, _receiver: Address, _guid: BytesN<32>, _msg: Bytes) {}
}

struct Setup<'a> {
    env: Env,
    owner: Address,
    client: RegistryOAppClient<'a>,
    peer_eth: BytesN<32>,
    peer_sol: BytesN<32>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let owner = Address::generate(&env);
    let endpoint = env.register(MockEndpoint, ());
    let oapp = env.register(RegistryOApp, (&owner, &endpoint));
    let client = RegistryOAppClient::new(&env, &oapp);
    let peer_eth = BytesN::from_array(&env, &[0xE1; 32]);
    let peer_sol = BytesN::from_array(&env, &[0x50; 32]);
    let s = Setup { env, owner, client, peer_eth, peer_sol };
    set_peer(&s, ETH_EID, &s.peer_eth);
    set_peer(&s, SOL_EID, &s.peer_sol);
    s
}

fn set_peer(s: &Setup, eid: u32, peer: &BytesN<32>) {
    let p = Some(peer.clone());
    s.env.mock_auths(&[MockAuth {
        address: &s.owner,
        invoke: &MockAuthInvoke {
            contract: &s.client.address,
            fn_name: "set_peer",
            args: (&eid, &p, &s.owner).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.set_peer(&eid, &p, &s.owner);
}

fn payload(env: &Env, chain_id: u8) -> Bytes {
    let mut b = [0u8; MESSAGE_LEN as usize];
    b[..32].copy_from_slice(&[0xAA; 32]);
    b[32] = chain_id;
    b[33..].copy_from_slice(&[0xBB; 32]);
    Bytes::from_array(env, &b)
}

fn commitment(env: &Env, msg: &Bytes) -> BytesN<32> {
    env.crypto().sha256(msg).into()
}

fn deliver(s: &Setup, src_eid: u32, sender: &BytesN<32>, msg: &Bytes) {
    let executor = Address::generate(&s.env);
    let origin = Origin { src_eid, sender: sender.clone(), nonce: 1 };
    let guid = BytesN::from_array(&s.env, &[7u8; 32]);
    let extra = Bytes::new(&s.env);
    s.env.mock_auths(&[MockAuth {
        address: &executor,
        invoke: &MockAuthInvoke {
            contract: &s.client.address,
            fn_name: "lz_receive",
            args: (&executor, &origin, &guid, msg, &extra, 0i128).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.lz_receive(&executor, &origin, &guid, msg, &extra, &0i128);
}

#[test]
fn eth_message_is_recorded() {
    let s = setup();
    let msg = payload(&s.env, CHAIN_ETH);
    s.env.ledger().set_timestamp(1_000);
    deliver(&s, ETH_EID, &s.peer_eth, &msg);
    let c = commitment(&s.env, &msg);
    assert_eq!(s.client.is_registered(&c), Some(1_000));
    assert_eq!(s.client.source_eid(&c), Some(ETH_EID));
}

#[test]
fn solana_message_is_recorded() {
    let s = setup();
    let msg = payload(&s.env, CHAIN_SOLANA);
    deliver(&s, SOL_EID, &s.peer_sol, &msg);
    assert_eq!(s.client.source_eid(&commitment(&s.env, &msg)), Some(SOL_EID));
}

#[test]
fn unknown_commitment_is_none() {
    let s = setup();
    let c = BytesN::from_array(&s.env, &[9u8; 32]);
    assert_eq!(s.client.is_registered(&c), None);
}

#[test]
fn redelivery_keeps_first_timestamp() {
    let s = setup();
    let msg = payload(&s.env, CHAIN_ETH);
    s.env.ledger().set_timestamp(1_000);
    deliver(&s, ETH_EID, &s.peer_eth, &msg);
    s.env.ledger().set_timestamp(5_000);
    deliver(&s, ETH_EID, &s.peer_eth, &msg);
    assert_eq!(s.client.is_registered(&commitment(&s.env, &msg)), Some(1_000));
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn short_payload_rejected() {
    let s = setup();
    deliver(&s, ETH_EID, &s.peer_eth, &Bytes::from_array(&s.env, &[1u8; 64]));
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn long_payload_rejected() {
    let s = setup();
    deliver(&s, ETH_EID, &s.peer_eth, &Bytes::from_array(&s.env, &[1u8; 66]));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn unknown_chain_id_rejected() {
    let s = setup();
    deliver(&s, ETH_EID, &s.peer_eth, &payload(&s.env, 3));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn chain_id_zero_rejected() {
    let s = setup();
    deliver(&s, ETH_EID, &s.peer_eth, &payload(&s.env, 0));
}

#[test]
#[should_panic(expected = "Error(Contract, #2002)")]
fn wrong_peer_rejected() {
    let s = setup();
    let rogue = BytesN::from_array(&s.env, &[0xFF; 32]);
    deliver(&s, ETH_EID, &rogue, &payload(&s.env, CHAIN_ETH));
}

#[test]
#[should_panic(expected = "Error(Contract, #2001)")]
fn unconfigured_source_chain_rejected() {
    let s = setup();
    deliver(&s, 12345, &s.peer_eth, &payload(&s.env, CHAIN_ETH));
}

// Same vector as backend/tests/test_registry_onchain.py::GOLDEN. If either side changes how the
// commitment is built, one of the two tests breaks.
#[test]
fn commitment_matches_python_golden_vector() {
    let env = Env::default();
    let msg = payload(&env, CHAIN_ETH);
    let got = commitment(&env, &msg).to_array();
    let hex: std::string::String = got.iter().map(|b| std::format!("{:02x}", b)).collect();
    assert_eq!(hex, "85fc8051287bece2c6c5a9ce4ef6373093360ac0d1a81ffbb28b23b05b049ace");
}

// --- caveat coverage -----------------------------------------------------------------------

// An endpoint whose `clear` always rejects, standing in for "message not DVN-verified".
#[contract]
pub struct RejectingEndpoint;

#[contractimpl]
impl RejectingEndpoint {
    pub fn set_delegate(_env: Env, _oapp: &Address, _delegate: &Option<Address>) {}
    pub fn native_token(env: Env) -> Address {
        Address::generate(&env)
    }
    pub fn clear(_env: Env, _caller: Address, _origin: Origin, _receiver: Address, _guid: BytesN<32>, _msg: Bytes) {
        panic!("not verified");
    }
}

#[test]
fn endpoint_rejection_stores_nothing() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let endpoint = env.register(RejectingEndpoint, ());
    let oapp = env.register(RegistryOApp, (&owner, &endpoint));
    let client = RegistryOAppClient::new(&env, &oapp);
    let peer = BytesN::from_array(&env, &[0xE1; 32]);
    let p = Some(peer.clone());
    env.mock_auths(&[MockAuth {
        address: &owner,
        invoke: &MockAuthInvoke {
            contract: &oapp,
            fn_name: "set_peer",
            args: (&ETH_EID, &p, &owner).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.set_peer(&ETH_EID, &p, &owner);

    let msg = payload(&env, CHAIN_ETH);
    let executor = Address::generate(&env);
    let origin = Origin { src_eid: ETH_EID, sender: peer, nonce: 1 };
    let guid = BytesN::from_array(&env, &[7u8; 32]);
    let extra = Bytes::new(&env);
    env.mock_auths(&[MockAuth {
        address: &executor,
        invoke: &MockAuthInvoke {
            contract: &oapp,
            fn_name: "lz_receive",
            args: (&executor, &origin, &guid, &msg, &extra, 0i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_lz_receive(&executor, &origin, &guid, &msg, &extra, &0i128).is_err());
    assert_eq!(client.is_registered(&commitment(&env, &msg)), None);
}

#[test]
fn ttl_can_be_extended_and_unknown_returns_false() {
    let s = setup();
    let msg = payload(&s.env, CHAIN_ETH);
    deliver(&s, ETH_EID, &s.peer_eth, &msg);
    let c = commitment(&s.env, &msg);
    // Age the ledger so the entry's TTL has decayed below the bump threshold.
    let seq = s.env.ledger().sequence();
    s.env.ledger().set_sequence_number(seq + 100 * crate::BUMP_THRESHOLD / 30);
    assert!(s.client.extend_registration_ttl(&c));
    let ttl = s.env.as_contract(&s.client.address, || {
        s.env.storage().persistent().get_ttl(&crate::RegKey::Commitment(c.clone()))
    });
    assert!(ttl >= crate::BUMP_TO - 1, "ttl {ttl}");
    assert!(!s.client.extend_registration_ttl(&BytesN::from_array(&s.env, &[9u8; 32])));
}

#[test]
fn set_peer_without_auth_is_rejected() {
    let s = setup();
    s.env.mock_auths(&[]);
    let rogue = Address::generate(&s.env);
    let p = Some(BytesN::from_array(&s.env, &[0x11; 32]));
    assert!(s.client.try_set_peer(&ETH_EID, &p, &rogue).is_err());
    // The pinned peer is unchanged.
    assert_eq!(s.client.peer(&ETH_EID), Some(s.peer_eth.clone()));
}

#[test]
fn grant_role_without_auth_is_rejected() {
    let s = setup();
    s.env.mock_auths(&[]);
    let rogue = Address::generate(&s.env);
    let role = soroban_sdk::Symbol::new(&s.env, "authorizer");
    assert!(s.client.try_grant_role(&rogue, &role, &rogue).is_err());
}
