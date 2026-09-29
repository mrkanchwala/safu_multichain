// Cross-chain stakers and backers (EVM / Solana owners), 2026-09-24, multichain build.
//
// An EVM or Solana wallet never holds a Stellar account. Its USDC crosses on Circle's CCTP to one of
// our adapters, which creates the owner's own safu-account on Stellar and stakes (stake adapter) or
// backs (back adapter) with it. Every later action on that account is authorised by the OWNER's own
// signature over the Soroban auth payload; our relayer (backend/relay.py) only pays the Stellar fee.
//
// Mirrors the Python that proved each path live on testnet: scripts/e2e/groups_b.py (EVM burn),
// solana_cctp.burn_with_hook (Solana burn), e2e_lib.EvmOwner / SolanaOwner (owner signatures).

import { StrKey, nativeToScVal, xdr } from "@stellar/stellar-sdk";
import { createPublicClient, encodeFunctionData, http, parseAbi } from "viem";
import { mainnet, sepolia } from "viem/chains";
import { IS_MAINNET } from "./pool";
import {
  AccountRole,
  address as solAddress,
  appendTransactionMessageInstruction,
  compileTransaction,
  createSolanaRpc,
  createTransactionMessage,
  generateKeyPair,
  getAddressDecoder,
  getAddressEncoder,
  getAddressFromPublicKey,
  getBase64EncodedWireTransaction,
  getProgramDerivedAddress,
  getPublicKeyFromAddress,
  getSignatureFromTransaction,
  getTransactionDecoder,
  getTransactionEncoder,
  partiallySignTransaction,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  verifySignature,
} from "@solana/kit";
import type { Address as SolAddress, SignatureBytes, Transaction } from "@solana/kit";

import type { AppClient } from "./client";
import {
  BACK_ADAPTER_ID,
  CCTP_DOMAIN,
  CCTP_FINALITY_FAST,
  EVM_MESSAGE_TRANSMITTER_V2,
  EVM_RPC_URL,
  EVM_TOKEN_MESSENGER_V2,
  EVM_USDC,
  SOLANA_MESSAGE_TRANSMITTER,
  SOLANA_RPC_URL,
  SOLANA_TOKEN_MESSENGER_MINTER,
  SOLANA_USDC_MINT,
  STAKE_ADAPTER_ID,
} from "./network";
import { readContract } from "./soroban";

export type Mode = "stake" | "back";
export const adapterFor = (mode: Mode) => (mode === "stake" ? STAKE_ADAPTER_ID : BACK_ADAPTER_ID);

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const unhex = (h: string) => Uint8Array.from((h.replace(/^0x/, "").match(/../g) ?? []).map((x) => parseInt(x, 16)));

// --- which safu-account belongs to this wallet ---------------------------------------------------

function ownerKey(kind: "evm" | "solana", owner: string): xdr.ScVal {
  const raw = kind === "evm" ? unhex(owner) : new Uint8Array(getAddressEncoder().encode(solAddress(owner)));
  return xdr.ScVal.scvVec([xdr.ScVal.scvSymbol(kind === "evm" ? "Evm" : "Solana"), nativeToScVal(raw, { type: "bytes" })]);
}

/** The safu-account the adapter assigns to this owner. Deterministic: it exists as an address even
 *  before the first deposit creates the contract. Stake and back use different adapters, so a
 *  wallet has one account for staking and a separate one for backing. */
export async function safuAccountFor(kind: "evm" | "solana", owner: string, mode: Mode): Promise<string> {
  const domain = kind === "evm" ? CCTP_DOMAIN.evm : CCTP_DOMAIN.solana;
  const v = await readContract<unknown>(adapterFor(mode), "account_address", [
    nativeToScVal(domain, { type: "u32" }),
    ownerKey(kind, owner),
  ]);
  return String(v);
}

// --- EVM burn (Ethereum -> Stellar) -----------------------------------------------------------------

const ERC20 = parseAbi(["function approve(address spender, uint256 amount) returns (bool)",
  "function balanceOf(address owner) view returns (uint256)"]);
const MESSENGER = parseAbi([
  "function depositForBurn(uint256 amount, uint32 destinationDomain, bytes32 mintRecipient, address burnToken, bytes32 destinationCaller, uint256 maxFee, uint32 minFinalityThreshold)",
]);
const evmClient = createPublicClient({ chain: IS_MAINNET ? mainnet : sepolia, transport: http(EVM_RPC_URL) });

export async function evmUsdcBalance(owner: string): Promise<bigint> {
  return evmClient.readContract({ address: EVM_USDC, abi: ERC20, functionName: "balanceOf", args: [owner as `0x${string}`] });
}

async function evmSend(client: AppClient, to: string, data: `0x${string}`): Promise<`0x${string}`> {
  const hash = (await client.evmRequest("eth_sendTransaction", [{ from: client.address, to, data }])) as `0x${string}`;
  const receipt = await evmClient.waitForTransactionReceipt({ hash, timeout: 300_000 });
  if (receipt.status !== "success") throw new Error("Your Ethereum transaction failed. Check you have enough ETH for the fee, then try again.");
  return hash;
}

/** approve + depositForBurn to the adapter (mintRecipient AND destinationCaller = the adapter, so
 *  only our adapter can finish it). `usdc6` has 6 decimals, Ethereum USDC's own scale. */
export async function evmBurnToAdapter(client: AppClient, usdc6: bigint, mode: Mode): Promise<string> {
  const adapter32 = ("0x" + hex(StrKey.decodeContract(adapterFor(mode)))) as `0x${string}`;
  await evmSend(client, EVM_USDC, encodeFunctionData({
    abi: ERC20, functionName: "approve", args: [EVM_TOKEN_MESSENGER_V2, usdc6] }));
  return evmSend(client, EVM_TOKEN_MESSENGER_V2, encodeFunctionData({
    abi: MESSENGER, functionName: "depositForBurn",
    args: [usdc6, CCTP_DOMAIN.stellar, adapter32, EVM_USDC, adapter32, usdc6 / 1000n, CCTP_FINALITY_FAST],
  }));
}

// --- Solana burn with hook (devnet -> Stellar) ---------------------------------------------------

const TOKEN_PROGRAM = solAddress("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ATA_PROGRAM = solAddress("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM = solAddress("11111111111111111111111111111111");
const MT = solAddress(SOLANA_MESSAGE_TRANSMITTER);
const TMM = solAddress(SOLANA_TOKEN_MESSENGER_MINTER);
const MINT = solAddress(SOLANA_USDC_MINT);
const DISC_BURN_WITH_HOOK = Uint8Array.from([111, 245, 62, 131, 204, 108, 223, 155]);
const solRpc = createSolanaRpc(SOLANA_RPC_URL);
const enc = getAddressEncoder();
const bytes = (a: SolAddress) => new Uint8Array(enc.encode(a));

async function pda(seeds: (string | Uint8Array)[], program: SolAddress): Promise<SolAddress> {
  const [a] = await getProgramDerivedAddress({ programAddress: program, seeds });
  return a;
}

export async function solanaAta(owner: string): Promise<{ ata: SolAddress; bump: number }> {
  const [ata, bump] = await getProgramDerivedAddress({
    programAddress: ATA_PROGRAM, seeds: [bytes(solAddress(owner)), bytes(TOKEN_PROGRAM), bytes(MINT)] });
  return { ata, bump };
}

export async function solanaUsdcBalance(owner: string): Promise<bigint> {
  const { ata } = await solanaAta(owner);
  try {
    const r = await solRpc.getTokenAccountBalance(ata, { commitment: "confirmed" }).send();
    return BigInt(r.value.amount);
  } catch {
    return 0n;
  }
}

function le(n: bigint | number, size: 4 | 8): Uint8Array {
  const b = new Uint8Array(size);
  const v = new DataView(b.buffer);
  if (size === 8) v.setBigUint64(0, BigInt(n), true);
  else v.setUint32(0, Number(n), true);
  return b;
}

const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};

/** CCTP v2 `deposit_for_burn_with_hook`: hook data = the owner's own USDC ATA + bump, which the
 *  adapter stores as the payout account (cctp-adapter solana_payout_account). */
export async function solanaBurnToAdapter(client: AppClient, usdc6: bigint, mode: Mode): Promise<string> {
  if (!client.address) throw new Error("Connect a Solana wallet first.");
  const owner = solAddress(client.address);
  const adapter32 = StrKey.decodeContract(adapterFor(mode));
  const { ata, bump } = await solanaAta(client.address);
  const hook = concat(bytes(ata), Uint8Array.from([bump]));
  const data = concat(DISC_BURN_WITH_HOOK, le(usdc6, 8), le(CCTP_DOMAIN.stellar, 4), adapter32, adapter32,
    le(usdc6 / 1000n, 8), le(CCTP_FINALITY_FAST, 4), le(hook.length, 4), hook);

  const eventKeys = await generateKeyPair();
  const event = await getAddressFromPublicKey(eventKeys.publicKey);
  const accounts = [
    { address: owner, role: AccountRole.WRITABLE_SIGNER }, // owner + event_rent_payer (same key)
    { address: owner, role: AccountRole.WRITABLE_SIGNER },
    { address: await pda(["sender_authority"], TMM), role: AccountRole.READONLY },
    { address: ata, role: AccountRole.WRITABLE },
    { address: await pda(["denylist_account", bytes(owner)], TMM), role: AccountRole.READONLY },
    { address: await pda(["message_transmitter"], MT), role: AccountRole.WRITABLE },
    { address: await pda(["token_messenger"], TMM), role: AccountRole.READONLY },
    { address: await pda(["remote_token_messenger", String(CCTP_DOMAIN.stellar)], TMM), role: AccountRole.READONLY },
    { address: await pda(["token_minter"], TMM), role: AccountRole.READONLY },
    { address: await pda(["local_token", bytes(MINT)], TMM), role: AccountRole.WRITABLE },
    { address: MINT, role: AccountRole.WRITABLE },
    { address: event, role: AccountRole.WRITABLE_SIGNER },
    { address: MT, role: AccountRole.READONLY },
    { address: TMM, role: AccountRole.READONLY },
    { address: TOKEN_PROGRAM, role: AccountRole.READONLY },
    { address: SYSTEM, role: AccountRole.READONLY },
    { address: await pda(["__event_authority"], TMM), role: AccountRole.READONLY },
    { address: TMM, role: AccountRole.READONLY },
  ];
  const { value: blockhash } = await solRpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const message = pipe(
    createTransactionMessage({ version: "legacy" }),
    (m) => setTransactionMessageFeePayer(owner, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstruction({ programAddress: TMM, accounts, data }, m),
  );
  const partly = await partiallySignTransaction([eventKeys], compileTransaction(message));
  const signedBytes = await client.solanaSignTransaction(new Uint8Array(getTransactionEncoder().encode(partly)));
  const signed = getTransactionDecoder().decode(signedBytes) as Transaction;
  const sig = getSignatureFromTransaction(signed as Parameters<typeof getSignatureFromTransaction>[0]);
  await solRpc.sendTransaction(getBase64EncodedWireTransaction(signed), {
    encoding: "base64", preflightCommitment: "confirmed" }).send();
  for (let i = 0; i < 60; i++) {
    const st = await solRpc.getSignatureStatuses([sig]).send();
    const s = st.value[0];
    if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) {
      if (s.err) throw new Error("Your Solana transaction failed. Check you have enough SOL for the fee, then try again.");
      return sig;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error("Your Solana transaction wasn't confirmed in time. Check your wallet before trying again, so it doesn't happen twice.");
}

// --- relayer (backend/relay.py) ----------------------------------------------------------------

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const out = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(out.detail || `Request failed (${res.status})`);
  return out as T;
}

/** Waits for Circle to attest the burn, then has the relayer finish it on Stellar. Deposits use Fast
 *  Transfer, so this is usually seconds to a minute. */
export async function finishOnStellar(
  mode: Mode, kind: "evm" | "solana", burnTx: string, onWait?: (msg: string) => void, signal?: AbortSignal,
): Promise<string> {
  const deadline = Date.now() + 35 * 60_000;
  while (Date.now() < deadline) {
    if (signal?.aborted) throw new Error("Stopped.");
    const r = await post<{ status: string; account?: string; circle_status?: string }>("/relay/mint", {
      adapter: mode, source_domain: kind === "evm" ? CCTP_DOMAIN.evm : CCTP_DOMAIN.solana, burn_tx: burnTx });
    if (r.status === "done" && r.account) return r.account;
    onWait?.("Waiting for Circle to confirm (usually under a minute)...");
    await new Promise((res) => setTimeout(res, 15_000));
  }
  throw new Error("Circle's bridge is taking longer than usual. Your money is safe and will arrive on its own. Check back in a few minutes and don't send it again.");
}

export type RelayAction =
  | "approve_claim" | "claim_home" | "withdraw_home" | "exit_home" | "send_home"
  | "request_back_withdrawal" | "cancel_back_withdrawal" | "complete_back_withdrawal_home";

/** The safu-account accepts only a plain Ed25519 signature over the 32 payload bytes. Some Solana
 *  wallets sign a different format (live test 2026-09-25, Night wallet: every relayed action was
 *  refused, while the same action signed plainly passed). Checked here so the user gets a clear
 *  message instead of a generic failure. A browser without Ed25519 in WebCrypto skips the check
 *  and the backend still enforces it. */
async function checkSolanaSignature(owner: string, payload: Uint8Array, signatureHex: string): Promise<void> {
  let ok: boolean;
  try {
    const key = await getPublicKeyFromAddress(solAddress(owner));
    ok = await verifySignature(key, unhex(signatureHex) as SignatureBytes, payload);
  } catch {
    return;
  }
  if (!ok) throw new Error("WALLET_SIGNATURE_FORMAT");
}

/** One owner-signed action on the wallet's safu-account: the relayer builds it, the connected
 *  wallet signs the 32-byte auth payload, the relayer sends it. */
export async function relayAction(
  client: AppClient, account: string, action: RelayAction, extra: { claim_id?: string; amount?: string } = {},
): Promise<string> {
  const p = await post<{ id: string; payload: string }>("/relay/prepare", { action, account, ...extra });
  const payload = unhex(p.payload);
  const signature = await client.signMessage(payload);
  if (client.kind === "solana" && client.address) await checkSolanaSignature(client.address, payload, signature);
  const r = await post<{ tx_hash: string }>("/relay/submit", { id: p.id, signature });
  return r.tx_hash;
}

/** `mature_backing` needs no signature at all; the relayer just pays the fee. */
export async function relayMature(account: string): Promise<string> {
  return (await post<{ tx_hash: string }>("/relay/mature", { account })).tx_hash;
}

// --- money going home: the user's own wallet finishes it (docs/CCTP.md "Option A") -----------------
//
// Every "home" action (claim_home, withdraw_home, exit_home, send_home, complete_back_withdrawal_home)
// burns USDC on Stellar to the owner's own address / ATA. Circle attests it; then someone must submit
// it on the home chain. SAFU does not pay that gas (design decision, 2026-09-23): the connected wallet
// sends a normal `receiveMessage` (Sepolia) or `receive_message` (Solana devnet) transaction. If the
// user walks away, nothing is lost: anyone can finish the same message later.

export type HomeAction = "claim_home" | "withdraw_home" | "exit_home" | "send_home" | "complete_back_withdrawal_home";

const TRANSMITTER = parseAbi([
  "function receiveMessage(bytes message, bytes attestation) returns (bool)",
  "function usedNonces(bytes32 nonce) view returns (uint256)",
]);
const HEADER_LEN = 148; // CCTP v2 message header; the burn body follows
const DISC_RECEIVE = Uint8Array.from([38, 144, 127, 225, 31, 225, 238, 25]);

async function waitHomeMessage(stellarTx: string, onWait?: (msg: string) => void) {
  const deadline = Date.now() + 20 * 60_000;
  while (Date.now() < deadline) {
    const r = await post<{ status: string; message?: string; attestation?: string; destination_domain?: number }>(
      "/relay/home", { tx_hash: stellarTx });
    if (r.status === "ready" && r.message && r.attestation) return r as { message: string; attestation: string; destination_domain: number };
    onWait?.("Waiting for Circle to confirm the transfer home...");
    await new Promise((res) => setTimeout(res, 10_000));
  }
  throw new Error(`Circle's bridge is taking longer than usual. Your money is safe. Send this reference to the SAFU team to finish the transfer: ${stellarTx}`);
}

async function evmReceive(client: AppClient, message: string, attestation: string): Promise<string> {
  const nonce = ("0x" + message.slice(2 + 12 * 2, 2 + 44 * 2)) as `0x${string}`;
  const used = await evmClient.readContract({
    address: EVM_MESSAGE_TRANSMITTER_V2, abi: TRANSMITTER, functionName: "usedNonces", args: [nonce] });
  if (used !== 0n) return "Done. Your USDC was already delivered to your wallet.";
  return evmSend(client, EVM_MESSAGE_TRANSMITTER_V2, encodeFunctionData({
    abi: TRANSMITTER, functionName: "receiveMessage",
    args: [message as `0x${string}`, attestation as `0x${string}`] }));
}

/** One instruction, signed by the connected Solana wallet, sent and confirmed. */
async function solSendAndConfirm(
  client: AppClient,
  payer: ReturnType<typeof solAddress>,
  ix: Parameters<typeof appendTransactionMessageInstruction>[0],
): Promise<string> {
  const { value: blockhash } = await solRpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const msg = pipe(
    createTransactionMessage({ version: "legacy" }),
    (m) => setTransactionMessageFeePayer(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstruction(ix, m),
  );
  const signedBytes = await client.solanaSignTransaction(new Uint8Array(getTransactionEncoder().encode(compileTransaction(msg))));
  const signed = getTransactionDecoder().decode(signedBytes) as Transaction;
  const sig = getSignatureFromTransaction(signed as Parameters<typeof getSignatureFromTransaction>[0]);
  await solRpc.sendTransaction(getBase64EncodedWireTransaction(signed), { encoding: "base64", preflightCommitment: "confirmed" }).send();
  for (let i = 0; i < 60; i++) {
    const s = (await solRpc.getSignatureStatuses([sig]).send()).value[0];
    if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) {
      if (s.err) throw new Error("Couldn't set up your USDC account on Solana. Check you have enough SOL for the fee, then try again.");
      return sig;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error("Couldn't confirm your USDC account on Solana in time. Wait a minute and try again.");
}

async function solanaReceive(client: AppClient, messageHex: string, attestationHex: string): Promise<string> {
  if (!client.address) throw new Error("Connect a Solana wallet first.");
  const payer = solAddress(client.address);
  const message = unhex(messageHex);
  const attestation = unhex(attestationHex);
  const sourceDomain = new DataView(message.buffer, message.byteOffset).getUint32(4, false);
  const nonce = message.slice(12, 44);
  const body = message.slice(HEADER_LEN);
  const burnToken = body.slice(4, 36);
  const recipientTokenAccount = solAddress(getAddressDecoder().decode(body.slice(36, 68)));
  // receive_message fails if the recipient's USDC account doesn't exist yet (review W6). When it is the
  // connected wallet's own account, create it first in the same transaction (idempotent: a no-op if it
  // already exists). Anyone else's missing account can't be created from here without knowing its owner.
  // It goes in its OWN transaction: the receive is already near Solana's 1232-byte limit, and the extra
  // instruction pushed it over (1732 base64 bytes, found live 2026-09-25). Users who already have the
  // account see one prompt; only a missing account adds a second one.
  const recipientInfo = await solRpc.getAccountInfo(recipientTokenAccount, { encoding: "base64" }).send();
  if (!recipientInfo.value) {
    const { ata: payerAta } = await solanaAta(client.address);
    if (payerAta !== recipientTokenAccount) {
      throw new Error("Connect the Solana wallet this payout belongs to, so its USDC account can be set up.");
    }
    await solSendAndConfirm(client, payer, {
      programAddress: ATA_PROGRAM, // CreateIdempotent
      accounts: [
        { address: payer, role: AccountRole.WRITABLE_SIGNER },
        { address: recipientTokenAccount, role: AccountRole.WRITABLE },
        { address: payer, role: AccountRole.READONLY },
        { address: MINT, role: AccountRole.READONLY },
        { address: SYSTEM, role: AccountRole.READONLY },
        { address: TOKEN_PROGRAM, role: AccountRole.READONLY },
      ],
      data: Uint8Array.from([1]),
    });
  }

  const usedNonce = await pda(["used_nonce", nonce], MT);
  const existing = await solRpc.getAccountInfo(usedNonce, { encoding: "base64" }).send();
  if (existing.value) return "Done. Your USDC was already delivered to your wallet.";

  const tm = await pda(["token_messenger"], TMM);
  const tmInfo = await solRpc.getAccountInfo(tm, { encoding: "base64" }).send();
  if (!tmInfo.value) throw new Error("Circle's bridge isn't available on Solana right now. Your money is safe. Try again later.");
  const tmData = Uint8Array.from(atob(tmInfo.value.data[0]), (c) => c.charCodeAt(0));
  const feeRecipient = solAddress(getAddressDecoder().decode(tmData.slice(109, 141)));
  const { ata: feeRecipientAta } = await solanaAta(feeRecipient);

  const data = concat(DISC_RECEIVE, le(message.length, 4), message, le(attestation.length, 4), attestation);
  const accounts = [
    { address: payer, role: AccountRole.WRITABLE_SIGNER }, // payer
    { address: payer, role: AccountRole.WRITABLE_SIGNER }, // caller (destinationCaller is empty: anyone)
    { address: await pda(["message_transmitter_authority", bytes(TMM)], MT), role: AccountRole.READONLY },
    { address: await pda(["message_transmitter"], MT), role: AccountRole.READONLY },
    { address: usedNonce, role: AccountRole.WRITABLE },
    { address: TMM, role: AccountRole.READONLY },
    { address: SYSTEM, role: AccountRole.READONLY },
    { address: await pda(["__event_authority"], MT), role: AccountRole.READONLY },
    { address: MT, role: AccountRole.READONLY },
    // remaining accounts: the token messenger's receive handler
    { address: tm, role: AccountRole.READONLY },
    { address: await pda(["remote_token_messenger", String(sourceDomain)], TMM), role: AccountRole.READONLY },
    { address: await pda(["token_minter"], TMM), role: AccountRole.WRITABLE },
    { address: await pda(["local_token", bytes(MINT)], TMM), role: AccountRole.WRITABLE },
    { address: await pda(["token_pair", String(sourceDomain), burnToken], TMM), role: AccountRole.READONLY },
    { address: feeRecipientAta, role: AccountRole.WRITABLE },
    { address: recipientTokenAccount, role: AccountRole.WRITABLE },
    { address: await pda(["custody", bytes(MINT)], TMM), role: AccountRole.WRITABLE },
    { address: TOKEN_PROGRAM, role: AccountRole.READONLY },
    { address: await pda(["__event_authority"], TMM), role: AccountRole.READONLY },
    { address: TMM, role: AccountRole.READONLY },
  ];
  const { value: blockhash } = await solRpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const txMessage = pipe(
    createTransactionMessage({ version: "legacy" }),
    (m) => setTransactionMessageFeePayer(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstruction({ programAddress: MT, accounts, data }, m),
  );
  const unsigned = compileTransaction(txMessage);
  const signedBytes = await client.solanaSignTransaction(new Uint8Array(getTransactionEncoder().encode(unsigned)));
  const signed = getTransactionDecoder().decode(signedBytes) as Transaction;
  const sig = getSignatureFromTransaction(signed as Parameters<typeof getSignatureFromTransaction>[0]);
  await solRpc.sendTransaction(getBase64EncodedWireTransaction(signed), {
    encoding: "base64", preflightCommitment: "confirmed" }).send();
  for (let i = 0; i < 60; i++) {
    const st = await solRpc.getSignatureStatuses([sig]).send();
    const s = st.value[0];
    if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) {
      if (s.err) throw new Error("Your USDC couldn't be received on Solana. Your money is safe. Try again.");
      return sig;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error("Your USDC transfer on Solana wasn't confirmed in time. Check your wallet balance before trying again.");
}

/** Finish a Stellar -> home transfer in the connected wallet (it pays the gas). */
export async function finishHome(client: AppClient, stellarTx: string, onWait?: (msg: string) => void): Promise<string> {
  const m = await waitHomeMessage(stellarTx, onWait);
  onWait?.("Approve the last step in your wallet to receive the USDC.");
  if (m.destination_domain === CCTP_DOMAIN.evm) {
    if (client.kind !== "evm") throw new Error("Connect the Ethereum wallet this money belongs to.");
    return evmReceive(client, m.message, m.attestation);
  }
  if (client.kind !== "solana") throw new Error("Connect the Solana wallet this money belongs to.");
  return solanaReceive(client, m.message, m.attestation);
}

/** A home action through the relayer, then the home-chain receive in the user's own wallet. */
export async function relayHomeAction(
  client: AppClient, account: string, action: HomeAction, extra: { claim_id?: string; amount?: string } = {},
  onWait?: (msg: string) => void,
): Promise<string> {
  const stellarTx = await relayAction(client, account, action, extra);
  return finishHome(client, stellarTx, onWait);
}
