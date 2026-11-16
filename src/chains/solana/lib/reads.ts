// Chain reads for the panels: the pool, a stake, a backing, a claim. Layouts come from the IDL.
import { num, setting } from "./pool";
import { backerAddress, poolAddress, readAccount, stakeAddress } from "./program";
import type { PoolRecord } from "./program";

export type { PoolRecord };

/** Re-read after a confirmed send: now, then at 2 s and 5 s. The RPC node answering a read can be a
 *  moment behind the one that confirmed, so one read could still show the old state (B8 run 6). */
export function refreshSoon(bump: () => void): void {
  bump();
  setTimeout(bump, 2000);
  setTimeout(bump, 5000);
}

export type StakeRecord = {
  staker: string; beneficiary: string; amount: bigint; yield_index_at: bigint; staked_at: bigint;
  penalty_locked_until: bigint; forfeited: boolean; suspended: boolean;
  active_claim: string | null; reserved_claim: string | null;
};

export type BackerRecord = {
  backer: string; amount: bigint; pending_amount: bigint; pending_matures_at: bigint;
  withdraw_amount: bigint; withdraw_ready_at: bigint; yield_index_at: bigint; yield_owed: bigint;
};

export type ClaimStatus =
  | "Unused" | "Reserved" | "PendingTime" | "AwaitingApproval" | "Active" | "Completed" | "Cancelled" | "Expired";

export type ClaimRecord = {
  staker: string; hack_timestamp: bigint; entitlement: bigint; streamed: bigint; stake: bigint;
  cooldown_ends: bigint; vesting_ends: bigint; tier: number; status: ClaimStatus; approve_deadline: bigint;
  last_collected: bigint;
};

const PRECISION = num("YIELD_INDEX_PRECISION");

/** pool-core yields::owed. */
const owed = (amount: bigint, now: bigint, at: bigint) => (now > at ? (amount * (now - at)) / PRECISION : 0n);

export async function readPool(): Promise<PoolRecord | null> {
  return readAccount<PoolRecord>("Pool", await poolAddress());
}

export type MyStake = StakeRecord & { yieldOwed: bigint };

export async function readMyStake(pool: PoolRecord, staker: string): Promise<MyStake | null> {
  const s = await readAccount<StakeRecord>("StakeRecord", await stakeAddress(staker));
  if (!s) return null;
  return { ...s, yieldOwed: s.forfeited ? 0n : owed(s.amount, pool.staker_yield_index, s.yield_index_at) };
}

export type MyBacking = BackerRecord & { yieldOwed: bigint };

export async function readMyBacking(pool: PoolRecord, backer: string): Promise<MyBacking | null> {
  const b = await readAccount<BackerRecord>("BackerRecord", await backerAddress(backer));
  if (!b) return null;
  return { ...b, yieldOwed: b.yield_owed + owed(b.amount, pool.backer_yield_index, b.yield_index_at) };
}

export type MyClaim = { address: string; data: ClaimRecord };

/** The claim on this stake: the admitted one if any, else the queued one. */
export async function readMyClaim(stake: StakeRecord | null): Promise<MyClaim | null> {
  const a = stake?.active_claim ?? stake?.reserved_claim;
  if (!a) return null;
  const data = await readAccount<ClaimRecord>("Claim", a);
  return data ? { address: a, data } : null;
}

/** pool-core claim::vested: linear from cooldown end to vesting end. */
export function vested(c: ClaimRecord, now: number): bigint {
  const start = Number(c.cooldown_ends);
  const end = Number(c.vesting_ends);
  if (end <= start) return now >= end ? c.entitlement : 0n;
  const elapsed = Math.max(0, Math.min(now, end) - start);
  return (c.entitlement * BigInt(elapsed)) / BigInt(end - start);
}

/** Stake bounds: pool cap x min/max stake bps, all live settings (pool-core stake::bounds). */
export function stakeBounds(pool: PoolRecord): { min: bigint; max: bigint } {
  const bps = num("BPS_DENOMINATOR");
  const cap = setting(pool, "POOL_CAP");
  return { min: (cap * setting(pool, "MIN_STAKE_BPS")) / bps, max: (cap * setting(pool, "MAX_STAKE_BPS")) / bps };
}

/** What a withdrawal of `amount` leaves, or why it can't go (pool-core stake::check_withdraw). */
export function withdrawCheck(pool: PoolRecord, stake: bigint, amount: bigint): string | null {
  if (amount <= 0n) return "Enter an amount above zero.";
  if (amount > stake) return "That's more than your stake.";
  const left = stake - amount;
  const { min } = stakeBounds(pool);
  if (left > 0n && left < min) return `What stays must be at least ${fmtSol(min, 2)} SOL. Take it all out instead.`;
  return null;
}

export const LAMPORTS = 1_000_000_000n;

export function fmtSol(lamports: bigint, dp = 4): string {
  const neg = lamports < 0n;
  const v = neg ? -lamports : lamports;
  const whole = v / LAMPORTS;
  const frac = (v % LAMPORTS).toString().padStart(9, "0").slice(0, dp);
  return `${neg ? "-" : ""}${whole.toLocaleString()}${dp ? "." + frac : ""}`;
}

/** "1.5" -> lamports, exact (no float). Null on anything that isn't a plain positive decimal. */
export function parseSol(s: string): bigint | null {
  const m = /^\s*(\d*)(?:\.(\d{0,9}))?\s*$/.exec(s);
  if (!m || (!m[1] && !m[2])) return null;
  return BigInt(m[1] || "0") * LAMPORTS + BigInt((m[2] || "").padEnd(9, "0"));
}

export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 120) return `${s} seconds`;
  if (s < 7200) return `${Math.round(s / 60)} minutes`;
  if (s < 172800) return `${Math.round(s / 3600)} hours`;
  return `${Math.round(s / 86400)} days`;
}

// --- covered wallets (display list) --------------------------------------------------------------
// The program only stores wallet hashes, so the addresses shown here are kept in this browser, per
// staker (same as the multichain site). The backend's registry is the real list.

// `chain` is missing on rows saved before Ethereum wallets were added: those are Solana.
export type CoveredWallet = { chain?: string; wallet: string; registeredAt: number };
const storageKey = (owner: string) => `safu_solana_covered_wallets:${owner}`;

export function readCoveredWallets(owner: string): CoveredWallet[] {
  try {
    const raw = localStorage.getItem(storageKey(owner));
    return raw ? (JSON.parse(raw) as CoveredWallet[]) : [];
  } catch {
    return [];
  }
}

export function rememberCoveredWallet(owner: string, chain: string, wallet: string, registeredAt: number): CoveredWallet {
  const row = { chain, wallet, registeredAt: registeredAt * 1000 };
  try {
    const cur = readCoveredWallets(owner);
    if (!cur.some((w) => w.wallet === wallet)) localStorage.setItem(storageKey(owner), JSON.stringify([...cur, row]));
  } catch {
    // display only
  }
  return row;
}
