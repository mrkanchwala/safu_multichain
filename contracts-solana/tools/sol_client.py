"""Devnet client for registry-sender. Usage: sol_client.py <init_store|init_send_library|init_nonce|register> [--send]
Every step is simulated first; nothing is sent without --send. Uses the CLI's configured keypair as payer/admin."""
import base64
import hashlib
import json
import struct
import subprocess
import sys

from solders.hash import Hash
from solders.instruction import AccountMeta, Instruction
from solders.keypair import Keypair
from solders.message import Message
from solders.pubkey import Pubkey
from solders.transaction import Transaction

RPC = "https://api.devnet.solana.com"
EP = Pubkey.from_string("76y77prsiCMvXMjuoZ5VRrhG5qYBrUMYTE5WgHqgjEn6")
ULN = Pubkey.from_string("7a4WjyR8VZ7yZz5XJAKm39BUGn5iT9CKcv2pmG9tdXVH")
SYS = Pubkey.from_string("11111111111111111111111111111111")
CB = Pubkey.from_string("ComputeBudget111111111111111111111111111111")
EXEC_PROG = Pubkey.from_string("6doghB248px58JSSwG4qejQ46kFMW4AMj7vzJnWZHNZn")
DVN_PROG = Pubkey.from_string("HtEYV4xB4wvsj5fgTkcfuChYpvGYzgzwvNhgDZQNh7wW")
PF_PROG = Pubkey.from_string("8ahPGPjEbpgGaZx2NV1iG5Shj7TDwvsjkEDcGWjt94TP")
EXEC_CFG = Pubkey.from_string("AwrbHeCyniXaQhiJZkLhgWdUCteeWSGaSN1sTfLiY7xK")
DVN_CFG = Pubkey.from_string("4VDjp6XQaxoZf5RGwiPU9NR1EXSZn2TP4ATMmiSzLfhb")
PF_CFG = Pubkey.from_string("CSFsUupvJEQQd1F4SsXGACJaxQX4eropQMkGV2696eeQ")
ULN_SETTINGS = Pubkey.from_string("2XgGZG4oP29U3w5h4nTk1V2LFHL23zKDPJjs3psGzLKQ")
DEFAULT_ULN_SEND_CFG = Pubkey.from_string("BXmMZunhXPY6PyUu1ug37WuzXJP2izAtDQ4ASkgHo2Hr")
DEFAULT_EP_SEND_CFG = Pubkey.from_string("4Ka3faFVVuFnUaPQWPg3vQRv8yxVE5YUqUfdPCmAYR23")
SEND_LIB_INFO = Pubkey.from_string("526PeNZfw8kSnDU4nmzJFVJzJWNhwmZykEyJr5XWz5Fv")
DST_EID = 40600
# Stellar registry-oapp (CCI5OVTL...) as raw bytes
RECEIVER = bytes.fromhex("91d7566b474a3ff9d5ab105f840faebede2f690619e61f18794b3fe24b1ff60c")
PROGRAM_ID = Pubkey.from_json(open("/Users/murtazakanchwala/SAFU/safu_multichain/contracts-solana/registry-sender/target/deploy/registry_sender-keypair.json").read()) if False else None


def disc(name):
    return hashlib.sha256(f"global:{name}".encode()).digest()[:8]


def pda(seeds, prog):
    return Pubkey.find_program_address(seeds, prog)[0]


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    out = subprocess.check_output(["curl", "-s", "-X", "POST", "-H", "Content-Type: application/json", "-d", body, RPC])
    return json.loads(out)


def load_payer():
    path = subprocess.check_output(["solana", "config", "get"]).decode()
    kp = [l for l in path.splitlines() if l.startswith("Keypair Path")][0].split(": ")[1].strip()
    return Keypair.from_bytes(bytes(json.load(open(kp))))


def program_id():
    kp = json.load(open("/Users/murtazakanchwala/SAFU/safu_multichain/contracts-solana/registry-sender/target/deploy/registry_sender-keypair.json"))
    return Keypair.from_bytes(bytes(kp)).pubkey()


def run(ixs, payer, send):
    bh = rpc("getLatestBlockhash", [{"commitment": "confirmed"}])["result"]["value"]["blockhash"]
    msg = Message.new_with_blockhash(ixs, payer.pubkey(), Hash.from_string(bh))
    tx = Transaction([payer], msg, Hash.from_string(bh))
    raw = base64.b64encode(bytes(tx)).decode()
    print("tx size bytes:", len(bytes(tx)))
    sim = rpc("simulateTransaction", [raw, {"encoding": "base64", "sigVerify": False, "commitment": "confirmed"}])
    r = sim.get("result", {}).get("value") or {}
    print("SIM err:", r.get("err") or sim.get("error"), "| units:", r.get("unitsConsumed"))
    for line in (r.get("logs") or [])[-14:]:
        print("   ", line[:200])
    if r.get("err") or not send:
        return None
    res = rpc("sendTransaction", [raw, {"encoding": "base64", "skipPreflight": True}])
    print("SEND:", res.get("result") or res)
    return res.get("result")


def main():
    step, send = sys.argv[1], "--send" in sys.argv
    payer = load_payer()
    prog = program_id()
    store = pda([b"Store"], prog)
    eid = DST_EID.to_bytes(4, "big")
    registry = pda([b"OApp", bytes(store)], EP)
    ep_event = pda([b"__event_authority"], EP)
    print("program", prog, "| store PDA", store, "| payer", payer.pubkey())
    W = lambda k, s=False: AccountMeta(k, s, True)      # noqa: E731
    R = lambda k, s=False: AccountMeta(k, s, False)     # noqa: E731

    if step == "init_store":
        ix = Instruction(prog, bytes([0]) + bytes(payer.pubkey()), [W(payer.pubkey(), True), W(store), R(SYS), R(EP), W(registry), R(ep_event)])
    elif step == "init_send_library":
        cfg = pda([b"SendLibraryConfig", bytes(store), eid], EP)
        ix = Instruction(EP, disc("init_send_library") + bytes(store) + struct.pack("<I", DST_EID),
                         [W(payer.pubkey(), True), R(registry), W(cfg), R(SYS)])
    elif step == "init_nonce":
        nonce = pda([b"Nonce", bytes(store), eid, RECEIVER], EP)
        pending = pda([b"PendingNonce", bytes(store), eid, RECEIVER], EP)
        ix = Instruction(EP, disc("init_nonce") + bytes(store) + struct.pack("<I", DST_EID) + RECEIVER,
                         [W(payer.pubkey(), True), R(registry), W(nonce), W(pending), R(SYS)])
    elif step == "register":
        staker_hash = hashlib.sha256(b"GBC4WTU4D37OR5NSKSIPPAUMUFWTDPZIYKIWYSLNLQX6X5HVN3ZXI55N").digest()
        wallet_hash = hashlib.sha256(str(payer.pubkey()).encode()).digest()  # Solana wallets are case-sensitive
        options = struct.pack(">HBHB", 3, 1, 17, 1) + (200000).to_bytes(16, "big")
        data = bytes([1]) + staker_hash + wallet_hash + struct.pack("<Q", 20_000_000) + RECEIVER + struct.pack("<H", len(options)) + options
        send_cfg = pda([b"SendLibraryConfig", bytes(store), eid], EP)
        nonce = pda([b"Nonce", bytes(store), eid, RECEIVER], EP)
        uln_event = pda([b"__event_authority"], ULN)
        uln_send_cfg = pda([b"SendConfig", eid, bytes(store)], ULN)
        ep_settings = pda([b"Endpoint"], EP)
        accts = [W(payer.pubkey(), True), R(store), R(EP),
                 R(ULN), R(send_cfg), R(DEFAULT_EP_SEND_CFG), R(SEND_LIB_INFO), R(ep_settings), W(nonce), R(ep_event), R(EP),
                 R(ULN_SETTINGS), R(uln_send_cfg), R(DEFAULT_ULN_SEND_CFG), W(payer.pubkey(), True), R(ULN), R(SYS), R(uln_event), R(ULN),
                 R(EXEC_PROG), W(EXEC_CFG), R(PF_PROG), R(PF_CFG),
                 R(DVN_PROG), W(DVN_CFG), R(PF_PROG), R(PF_CFG)]
        ix = Instruction(prog, data, accts)
        print("staker_hash", staker_hash.hex()[:16], "| wallet", payer.pubkey())
    else:
        raise SystemExit("unknown step")
    cu = Instruction(CB, bytes([2]) + struct.pack("<I", 600_000), [])
    run([cu, ix], payer, send)


if __name__ == "__main__":
    main()
