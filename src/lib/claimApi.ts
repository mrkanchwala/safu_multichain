// Talks to backend/app.py (2026-09-19) -- the demo claim-preparation API, NOT
// the Soroban contract. This produces a signed claim; it never broadcasts
// anything on-chain. See that file's own header for what's real vs demo-mode
// here (the response's `demoMode` says so per field, shown in ClaimFilePanel).
//
// tier + hackTimestamp REMOVED 2026-09-23 (message v1 -> v2) -- the backend
// measures the tier from the covered wallet's chain history and reads the
// hack time from the transaction itself. Neither is caller-supplied anymore.
//
// The wallet is auto-matched, not sent -- `staker` (the connected wallet) and
// `chain` are enough; the backend checks the tx against the staker's own
// registered covered wallets and reports which one matched (`wallet` in the
// RESPONSE). See claim.py's own docstring for the full reasoning.
//
// asset/lossAmount/entitlementUsdc REMOVED 2026-09-20 -- these used to be
// typed in and the pool just trusted them, capped only by the tier ceiling.
// Now derived server-side from the real transaction (backend/auto_loss.py).
//
// Caller-identity check ADDED 2026-09-20 (CSO HIGH fix, v2 same day) --
// both endpoints used to accept any `staker` address with no proof of
// ownership. v1 signed a plain message via StellarWalletsKit.signMessage;
// Albedo (the wallet this whole demo is built around) hard-refuses that
// call ("does not support the signMessage function"), so v1 broke covered-
// wallet registration and claim filing for exactly the wallet that matters
// most here. Every wallet in the kit DOES implement signTransaction
// (staking already proved this live), so the proof is now a throwaway
// SIGNED TRANSACTION that is never submitted -- same shape as a SEP-10
// web-auth challenge, for the same reason SEP-10 exists.
//
// The "transaction": source=staker, sequence=0 (Account(staker, -1) built
// locally -- sequence 0 can never be a real account's valid next sequence,
// so this can never be broadcast successfully even if the XDR leaked), one
// ManageData op named "safu_auth" whose value is the SHA-256 digest of the
// same canonical message v1 signed directly (32 bytes, fits ManageData's
// 64-byte limit; the full message does not). This must stay in lockstep
// with backend/app.py's `_verify_signed_request`.

import { Account, Operation, TransactionBuilder } from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit";

import { NETWORK_PASSPHRASE } from "./network";
import type { ChainId } from "./network";

async function sha256(message: string): Promise<Uint8Array> {
  const bytes = new TextEncoder().encode(message);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return new Uint8Array(digest);
}

async function signAuthProof(staker: string, message: string): Promise<string> {
  const digest = await sha256(message);
  const account = new Account(staker, "-1");
  const tx = new TransactionBuilder(account, { fee: "100", networkPassphrase: NETWORK_PASSPHRASE })
    .addOperation(Operation.manageData({ name: "safu_auth", value: digest }))
    .setTimeout(300)
    .build();
  const { signedTxXdr } = await StellarWalletsKit.signTransaction(tx.toXDR(), {
    networkPassphrase: NETWORK_PASSPHRASE,
    address: staker,
  });
  return signedTxXdr;
}

// --- who signs (2026-09-24) -----------------------------------------------------------------------
// A Stellar staker signs the throwaway-transaction proof above. An EVM/Solana staker's `staker` is
// their safu-account (a contract, which can't sign messages), so their own wallet signs the plain
// message and the backend checks it against the account's owner (backend/owner_auth.py).
import type { AppClient } from "./client";

async function signFor(client: AppClient, staker: string, message: string): Promise<Record<string, string>> {
  if (client.kind === "stellar") return { signature: await signAuthProof(staker, message) };
  if (client.kind === "evm" || client.kind === "solana") {
    if (!client.address) throw new Error("Connect a wallet first.");
    return { signature: await client.signMessage(message), owner_chain: client.kind, owner: client.address };
  }
  throw new Error("Connect a wallet first.");
}

export type ClaimEntry = { chain: ChainId; txHash: string };

export type FileClaimResult = {
  claimIdHex: string;
  wallet: string;
  staker: string;
  entitlementUsdc: number;
  tier: string;
  signatureHex: string;
  deadline: number;
  demoMode: Record<string, string>;
  broadcastResult: string;
};

export class ClaimApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function postJson(path: string, body: unknown) {
  const res = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const out = await res.json().catch(() => ({}));
  if (!res.ok) throw new ClaimApiError(res.status, out.detail || `Request failed (${res.status})`);
  return out;
}

/** v3 (2026-09-24): ONE claim over drains on up to three chains -- one entry per chain, the real
 *  shape of a stolen seed phrase. Message format MUST stay byte-identical with backend/app.py. */
export async function fileClaim(client: AppClient, staker: string, entries: ClaimEntry[]): Promise<FileClaimResult> {
  const clean = entries.map((e) => ({ chain: e.chain, tx_hash: e.txHash.trim() }));
  const timestamp = Math.floor(Date.now() / 1000);
  const canon = clean.map((e) => `${e.chain}:${e.tx_hash}`).join(",");
  const message = `SAFU_CLAIM:v3:${staker}:${canon}:${timestamp}`;
  const body = await postJson("/claim", { staker, entries: clean, timestamp, ...(await signFor(client, staker, message)) });
  return {
    claimIdHex: body.claim_id_hex,
    wallet: body.wallet,
    staker: body.staker,
    entitlementUsdc: body.entitlement_usdc,
    tier: body.tier,
    signatureHex: body.signature_hex,
    deadline: body.deadline,
    demoMode: body.demo_mode,
    broadcastResult: body.broadcast_result,
  };
}

// Covered wallets (ownership proof, 2026-09-23): step 1 registers straight away only on a same-chain
// link; otherwise it returns a tiny amount the covered wallet must send TO ITSELF, then step 2
// (/covered-wallets/confirm) finds that transfer on-chain and registers. No list endpoint, by design.
export type CoveredWalletResponse = {
  staker: string;
  chain: string;
  wallet: string;
  status: "registered" | "send_required" | "not_found_yet";
  registered_at?: number | null;
  method?: string | null;
  asset?: string | null;
  amount?: string | null;
  send_to?: string | null;
  expires_at?: number | null;
  proof_tx?: string | null;
  onchain?: string | null;
};

export async function addCoveredWallet(
  client: AppClient, staker: string, chain: string, wallet: string,
): Promise<CoveredWalletResponse> {
  const timestamp = Math.floor(Date.now() / 1000);
  const message = `SAFU_COVERED_WALLET:v1:${staker}:${chain}:${wallet}:${timestamp}`;
  return postJson("/covered-wallets", { staker, chain, wallet, timestamp, ...(await signFor(client, staker, message)) });
}

export async function confirmCoveredWallet(staker: string, chain: string, wallet: string): Promise<CoveredWalletResponse> {
  return postJson("/covered-wallets/confirm", { staker, chain, wallet });
}
