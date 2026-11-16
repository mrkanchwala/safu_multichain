// Talks to backend/colosseum_app.py (the claim API). Message formats MUST stay byte-identical with it:
//   SAFU_SOLANA_COVERED_WALLET:v2:{staker}:{chain}:{wallet}:{ts}
//   SAFU_SOLANA_CLAIM:v1:{staker}:{tx1,tx2,...}:{ts}
// The staking wallet signs each one (Solana signMessage). Nothing about the payout is typed in: the
// backend reads the loss, the hack time and the tier from the chain, and the oracle submits the claim.
import type { AppClient } from "./client";
import { POOL } from "./pool";

/** Chains a covered wallet can be on. Ethereum is Sepolia while the pool is on devnet. */
export type CoverChain = "solana" | "sepolia";

export type CoveredWalletResponse = {
  staker: string;
  wallet: string;
  chain: CoverChain;
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

export type FileClaimResult = {
  claim_address: string;
  claim_status: string;
  tx_signature: string;
  staker: string;
  wallets: string[];
  entitlement_lamports: number;
  tier: string;
  demo_mode: Record<string, string>;
};

export class ClaimApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const out = await res.json().catch(() => ({}));
  if (!res.ok) throw new ClaimApiError(res.status, typeof out.detail === "string" ? out.detail : `Request failed (${res.status})`);
  return out as T;
}

const now = () => Math.floor(Date.now() / 1000);

export async function addCoveredWallet(
  client: AppClient,
  staker: string,
  chain: CoverChain,
  wallet: string,
): Promise<CoveredWalletResponse> {
  const timestamp = now();
  const signature = await client.signMessage(`SAFU_SOLANA_COVERED_WALLET:v2:${staker}:${chain}:${wallet}:${timestamp}`);
  return postJson(`${POOL.apiBase}/covered-wallets`, { staker, chain, wallet, timestamp, signature });
}

export function confirmCoveredWallet(staker: string, chain: CoverChain, wallet: string): Promise<CoveredWalletResponse> {
  return postJson(`${POOL.apiBase}/covered-wallets/confirm`, { staker, chain, wallet });
}

export async function fileClaim(client: AppClient, staker: string, txHashes: string[]): Promise<FileClaimResult> {
  const tx_hashes = txHashes.map((t) => t.trim());
  const timestamp = now();
  const signature = await client.signMessage(`SAFU_SOLANA_CLAIM:v1:${staker}:${tx_hashes.join(",")}:${timestamp}`);
  return postJson(`${POOL.apiBase}/claim`, { staker, tx_hashes, timestamp, signature });
}
