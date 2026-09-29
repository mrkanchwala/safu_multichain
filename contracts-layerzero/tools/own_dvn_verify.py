"""Testnet only: sign + submit a LayerZero `verify` through our OWN self-run DVN on Stellar.

Why this exists: the default LayerZero DVN was not verifying Sepolia -> Stellar-testnet messages.
This is a documented workaround, not a substitute: the message still travels through LayerZero's
endpoint and message library, but the required verifier is one we operate. Say so wherever it is shown.

usage: own_dvn_verify.py <packet.json> [--send]
Reads the DVN signer key from ../../.env.sepolia (DVN_SIGNER_PRIVATE_KEY). Never prints keys.
"""
import hashlib
import json
import os
import subprocess
import sys
import time

from stellar_sdk import Keypair, Network, SorobanServer, TransactionBuilder, scval, xdr

RPC = "https://soroban-testnet.stellar.org"
DVN = "CD26OZQRFADEJAURQ6TIBXQ46UDXAWU436MA47RJYRC6Y4KG6MTJ7KHE"
ULN = "CCMLPCAWCPIIMXOHJJKU3NZLOFTT2O6QTB2UUFPN6SEHLK35QRHVKKMB"
VID = 10600
CONFIRMATIONS = 2  # overridden per packet by packet['confirmations']
SOURCE_IDENTITY = "hack-lz-deployer"


def env_key(name: str) -> str:
    path = os.path.join(os.path.dirname(__file__), "..", "..", ".env.sepolia")
    for line in open(path):
        if line.startswith(name + "="):
            return line.split("=", 1)[1].strip()
    raise SystemExit(f"{name} missing")


def call_scval(header: bytes, payload_hash: bytes, confirmations: int = CONFIRMATIONS) -> xdr.SCVal:
    args = scval.to_vec(
        [scval.to_address(DVN), scval.to_bytes(header), scval.to_bytes(payload_hash), scval.to_uint64(confirmations)]
    )
    return scval.to_map(
        {scval.to_symbol("args"): args, scval.to_symbol("func"): scval.to_symbol("verify"), scval.to_symbol("to"): scval.to_address(ULN)}
    )


def sign_digest(digest_hex: str) -> bytes:
    out = subprocess.check_output(
        ["cast", "wallet", "sign", "--no-hash", "0x" + digest_hex, "--private-key", env_key("DVN_SIGNER_PRIVATE_KEY")]
    ).decode().strip()
    sig = bytes.fromhex(out[2:])
    assert len(sig) == 65, len(sig)
    if os.environ.get('LZ_V01') and sig[64] >= 27:
        sig = sig[:64] + bytes([sig[64] - 27])
    return sig


def main() -> None:
    pkt = json.load(open(sys.argv[1]))
    send = "--send" in sys.argv
    header, ph = bytes.fromhex(pkt["header"]), bytes.fromhex(pkt["payload_hash"])
    expiry = int(time.time()) + 3600

    calls_vec = scval.to_vec([call_scval(header, ph, int(pkt.get('confirmations', CONFIRMATIONS)))])
    server0 = SorobanServer(RPC)
    acct0 = server0.load_account(Keypair.from_secret(subprocess.check_output(["stellar", "keys", "show", SOURCE_IDENTITY]).decode().strip()).public_key)
    hb = TransactionBuilder(acct0, Network.TESTNET_NETWORK_PASSPHRASE, base_fee=1_000_000)
    # The guard signs ONE self-call (execute_transaction(calls)), not the inner verify call.
    outer = scval.to_map(
        {
            scval.to_symbol("args"): scval.to_vec([calls_vec]),
            scval.to_symbol("func"): scval.to_symbol("execute_transaction"),
            scval.to_symbol("to"): scval.to_address(DVN),
        }
    )
    hb.append_invoke_contract_function_op(DVN, "hash_call_data", [scval.to_uint32(VID), scval.to_uint64(expiry), scval.to_vec([outer])])
    hb.set_timeout(300)
    hsim = server0.simulate_transaction(hb.build())
    if hsim.error:
        print("HASH SIM ERROR:", hsim.error[:300])
        raise SystemExit(1)
    digest = xdr.SCVal.from_xdr(hsim.results[0].xdr).bytes.sc_bytes.hex()
    sig = sign_digest(digest)

    secret = subprocess.check_output(["stellar", "keys", "show", SOURCE_IDENTITY]).decode().strip()
    kp = Keypair.from_secret(secret)

    def make_auth_data(payload: bytes) -> xdr.SCVal:
        admin_msg = payload if os.environ.get("LZ_ADMIN_OVER") != "digest" else bytes.fromhex(digest)
        sender = scval.to_vec(
            [scval.to_symbol("Admin"), scval.to_bytes(kp.raw_public_key()), scval.to_bytes(kp.sign(admin_msg))]
        )
        return scval.to_map(
            {
                scval.to_symbol("expiration"): scval.to_uint64(expiry),
                scval.to_symbol("sender"): sender,
                scval.to_symbol("signatures"): scval.to_vec([scval.to_bytes(sig)]),
                scval.to_symbol("vid"): scval.to_uint32(VID),
            }
        )

    server = SorobanServer(RPC)
    acct = server.load_account(kp.public_key)

    def build(auth=None):
        b = TransactionBuilder(acct, Network.TESTNET_NETWORK_PASSPHRASE, base_fee=1_000_000)
        b.append_invoke_contract_function_op(DVN, "execute_transaction", [calls_vec], auth=auth)
        b.set_timeout(300)
        return b.build()

    tx = build()
    sim = server.simulate_transaction(tx)
    if sim.error:
        print("SIM1 ERROR:", sim.error[:400])
        raise SystemExit(1)
    entries = [xdr.SorobanAuthorizationEntry.from_xdr(a) for a in sim.results[0].auth]
    print("auth entries recorded:", len(entries), [e.credentials.type.name for e in entries])
    signed = []
    latest = server.get_latest_ledger().sequence
    net_id = hashlib.sha256(Network.TESTNET_NETWORK_PASSPHRASE.encode()).digest()
    for e in entries:
        kind = e.credentials.type
        if kind == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_V2:
            c = e.credentials.address_v2
            c.signature_expiration_ledger = xdr.Uint32(latest + 1000)
            pre = xdr.HashIDPreimage(
                type=xdr.EnvelopeType.ENVELOPE_TYPE_SOROBAN_AUTHORIZATION_WITH_ADDRESS,
                soroban_authorization_with_address=xdr.HashIDPreimageSorobanAuthorizationWithAddress(
                    network_id=xdr.Hash(net_id), nonce=c.nonce, signature_expiration_ledger=c.signature_expiration_ledger,
                    address=c.address, invocation=e.root_invocation),
            )
            c.signature = make_auth_data(hashlib.sha256(pre.to_xdr_bytes()).digest())
        elif kind == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS:
            c = e.credentials.address
            c.signature_expiration_ledger = xdr.Uint32(latest + 1000)
            pre = xdr.HashIDPreimage(
                type=xdr.EnvelopeType.ENVELOPE_TYPE_SOROBAN_AUTHORIZATION,
                soroban_authorization=xdr.HashIDPreimageSorobanAuthorization(
                    network_id=xdr.Hash(net_id), nonce=c.nonce, signature_expiration_ledger=c.signature_expiration_ledger,
                    invocation=e.root_invocation),
            )
            c.signature = make_auth_data(hashlib.sha256(pre.to_xdr_bytes()).digest())
        signed.append(e)

    tx2 = build(auth=signed)
    sim2 = server.simulate_transaction(tx2)
    if sim2.error:
        print("SIM2 ERROR:", sim2.error[:600])
        raise SystemExit(1)
    print("SIMULATION OK. min resource fee:", sim2.min_resource_fee)
    if not send:
        print("(dry run, not submitted; pass --send)")
        return
    acct = server.load_account(kp.public_key)  # fresh sequence: the builds above advanced the local one
    tx3 = server.prepare_transaction(build(auth=signed))
    tx3.sign(kp)
    resp = server.send_transaction(tx3)
    print("submitted:", resp.status, resp.hash)
    if resp.error_result_xdr:
        print("error_result_xdr:", resp.error_result_xdr)
    for _ in range(30):
        r = server.get_transaction(resp.hash)
        if r.status.name != "NOT_FOUND":
            print("result:", r.status.name)
            break
        time.sleep(2)


if __name__ == "__main__":
    main()
