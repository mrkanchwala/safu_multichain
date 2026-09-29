import { USDC_DECIMALS } from "./network";
import { addr, bytesn32, hexFromBytes, isLive, poolId, readContract } from "./soroban";

// WIRED TO THE LIVE POOL, 2026-09-19 (after the pool-read fix) -- these used to be Stage 1
// stubs returning fixed dummy values regardless of real chain state, a leftover
// from before the pool deployed. `isLive()` now gates each one exactly the way
// `actions.ts`'s write path already did; when live, every read below is a real
// Soroban simulation via `readContract` (soroban.ts), verified against the
// actual deployed pool (`CBGWPWU6...`) before writing this, not guessed from
// the Rust source alone -- see `get_stake`/`get_claim`'s real output shapes.
//
// Pruned 2026-09-18: MockPosition / MockSupplier / MockBacker and their reads
// (readUstryBalance, readPosition, readSupplier, readBacker, readInventory,
// readBackstopConfig, readMarket) were the old vault+backstop money-market
// shapes -- dead with the dropped money market. fmtUstry / maxBorrowRaw /
// maxSafeWithdrawRaw / collateralValueRaw / fmtPrice / PRICE_SCALE went with
// them (LTV/collateral concepts that no longer apply to a staker/depositor
// pool). `readActiveClaim` and its Claim types stay, flagged below.

// FIXED 2026-09-19 (A9): ClaimStatus now matches the REAL contract enum
// exactly (types.rs:246-266) -- 8 states, u32-valued, not the old vault's
// 7-state mock with different names.
//
// FIXED again 2026-09-19 (after the pool-read fix): the real `get_claim` struct, verified
// live against the deployed pool, has no `loss` or `payout` field -- those
// were invented for the old mock. The real fields used here are `entitlement`
// (what the oracle signed, capped at the tier ceiling) and `streamed` (how
// much of it has actually paid out so far).
export type MockClaim = { status: ClaimStatus; entitlement: bigint; streamed: bigint };

// `erasableSyntaxOnly` (tsconfig) disallows real TS enums (they emit runtime
// code) -- a const object + derived union type is the erasable equivalent.
// Values match the contract's #[repr(u32)] exactly, so a raw read never
// needs a name-remapping table.
export const ClaimStatus = {
  Unused: 0,
  Active: 1,
  Completed: 2,
  Cancelled: 3,
  Reserved: 4,
  PendingTime: 5,
  AwaitingApproval: 6,
  Expired: 7,
} as const;
export type ClaimStatus = (typeof ClaimStatus)[keyof typeof ClaimStatus];

const DUMMY_DELAY_MS = 250;
function delay<T>(value: T): Promise<T> {
  return new Promise((resolve) => setTimeout(() => resolve(value), DUMMY_DELAY_MS));
}

export async function readUsdcBalance(_rpc: unknown, _owner: string): Promise<bigint> {
  return delay(0n);
}

/** `get_total_staked` / `get_total_stakers` (storage.rs:182,206). */
export async function readPoolStats(
  _rpc: unknown,
): Promise<{ totalStaked: bigint; totalStakers: number }> {
  if (!isLive()) return delay({ totalStaked: 0n, totalStakers: 0 });
  const [totalStaked, totalStakers] = await Promise.all([
    readContract<bigint>(poolId(), "get_total_staked"),
    readContract<number>(poolId(), "get_total_stakers"),
  ]);
  return { totalStaked, totalStakers };
}

export type MyStake = {
  amount: bigint; // fixed admission-time principal -- the entitlement basis
  withdrawable: bigint; // principal + accrued yield -- get_withdrawable_amount
  yieldOwed: bigint; // r3: yield takeable now without unstaking -- get_staker_yield_owed
  activeClaimId: string | null;
};

/** `get_stake` + `get_withdrawable_amount` (lib.rs). `get_stake` returns
 * `Option<StakeRecord>` -- null/undefined on no stake, handled below. The
 * record's `active_claim_id` (falls back to `reserved_claim_id` if a claim
 * is sitting in `Reserved` status rather than fully `Active`) is what makes
 * `readActiveClaim` below possible with no separate backend lookup: the
 * contract already tracks "this staker's claim" on the stake itself. */
export async function readMyStake(_rpc: unknown, owner: string): Promise<MyStake | null> {
  if (!isLive()) return delay(null);
  const record = await readContract<Record<string, unknown> | null | undefined>(
    poolId(),
    "get_stake",
    [addr(owner)],
  );
  if (!record) return null;
  const [withdrawable, yieldOwed] = await Promise.all([
    readContract<bigint>(poolId(), "get_withdrawable_amount", [addr(owner)]),
    readContract<bigint>(poolId(), "get_staker_yield_owed", [addr(owner)]),
  ]);
  const claimBytes = (record.active_claim_id ?? record.reserved_claim_id) as
    | Uint8Array
    | null
    | undefined;
  return {
    amount: record.amount as bigint,
    withdrawable,
    yieldOwed,
    activeClaimId: claimBytes ? hexFromBytes(claimBytes) : null,
  };
}

/** No `get_claim`-by-wallet exists on-chain, by design -- `get_claim` takes a
 * claim id. This composes `readMyStake` (which surfaces the id from the
 * stake record itself) with `get_claim(claim_id)`, so the caller never needs
 * a separate claim-id store. Real field names verified live against the
 * deployed pool: `entitlement`, `streamed`, `status` -- see MockClaim's note. */
export async function readActiveClaim(
  rpc: unknown,
  owner: string,
): Promise<{ claimIdHex: string; data: MockClaim } | null> {
  if (!isLive()) return delay(null);
  const stake = await readMyStake(rpc, owner);
  if (!stake?.activeClaimId) return null;
  const claim = await readContract<Record<string, unknown> | null | undefined>(
    poolId(),
    "get_claim",
    [bytesn32(stake.activeClaimId)],
  );
  if (!claim) return null;
  return {
    claimIdHex: stake.activeClaimId,
    data: {
      status: claim.status as ClaimStatus,
      entitlement: claim.entitlement as bigint,
      streamed: claim.streamed as bigint,
    },
  };
}

/** `get_yield_index` (lib.rs). 1.0x (YIELD_INDEX_PRECISION) is the correct
 * value for a pool that has never realised yield -- NOT zero, which would
 * misread as "no stakes exist" -- so that stays the not-live fallback. */
export async function readYieldIndex(_rpc: unknown): Promise<bigint> {
  if (!isLive()) return delay(1_000_000_000_000n);
  return readContract<bigint>(poolId(), "get_yield_index");
}

export type CoveredWallet = { chain: string; wallet: string; registeredAt: number };

/**
 * WIRED 2026-09-19 -- `POST /covered-wallets` now exists (`backend/app.py`)
 * and writes to the real `backend/registry.py`-backed file, the same store
 * `/claim`'s auto-match checks against. There is deliberately NO matching
 * GET/list endpoint (standing privacy decision, `app.py`'s own comment: "no
 * list wallets by staker" -- revealing which wallets one staker covers across
 * chains is a cross-chain identity link, not just "their own data"). So the
 * DISPLAY list below stays local (localStorage, keyed per owner address) --
 * a convenience mirror of what you told the server, never re-fetched from it.
 * The registration-of-record that `/claim` actually checks lives server-side.
 */
function _storageKey(owner: string): string {
  return `safu_covered_wallets:${owner}`;
}

export class WalletAlreadyCovered extends Error {}

export async function readCoveredWallets(owner: string): Promise<CoveredWallet[]> {
  try {
    const raw = localStorage.getItem(_storageKey(owner));
    return delay(raw ? (JSON.parse(raw) as CoveredWallet[]) : []);
  } catch {
    return delay([]);
  }
}

/** Remember a registered covered wallet in the local display list (see the note above). */
export function rememberCoveredWallet(owner: string, chain: string, wallet: string, registeredAt: number): CoveredWallet {
  const local: CoveredWallet = { chain, wallet, registeredAt: registeredAt * 1000 };
  try {
    const raw = localStorage.getItem(_storageKey(owner));
    const existing = raw ? (JSON.parse(raw) as CoveredWallet[]) : [];
    if (!existing.some((w) => w.chain === chain && w.wallet === wallet)) {
      localStorage.setItem(_storageKey(owner), JSON.stringify([...existing, local]));
    }
  } catch {
    // Display cache only; the server-side registration already succeeded.
  }
  return local;
}

export function fromRaw(raw: bigint | number, decimals: number): number {
  return Number(raw) / 10 ** decimals;
}

export function toRaw(ui: number, decimals: number): bigint {
  return BigInt(Math.round(ui * 10 ** decimals));
}

export const fmtUsdc = (raw: bigint) => fromRaw(raw, USDC_DECIMALS).toFixed(2);

export type MyBacking = {
  amount: bigint; // matured: counts toward capacity
  pendingAmount: bigint; // deposited, not matured yet
  pendingMaturesAt: number; // unix seconds, 0 when nothing pending
  withdrawAmount: bigint; // open withdrawal request, 0 when none
  withdrawReadyAt: number; // unix seconds
  yieldOwed: bigint; // r3: yield takeable now -- get_backer_yield_owed
};

/** `get_backer` (lib.rs:277) -> Option<BackerRecord> (types.rs:350). */
export async function readMyBacking(owner: string): Promise<MyBacking | null> {
  if (!isLive()) return delay(null);
  const [r, yieldOwed] = await Promise.all([
    readContract<Record<string, unknown> | null | undefined>(poolId(), "get_backer", [addr(owner)]),
    readContract<bigint>(poolId(), "get_backer_yield_owed", [addr(owner)]),
  ]);
  if (!r) return null;
  const n = (k: string) => BigInt((r[k] as bigint | number | undefined) ?? 0);
  return {
    amount: n("amount"),
    pendingAmount: n("pending_amount"),
    pendingMaturesAt: Number(r.pending_matures_at ?? 0),
    withdrawAmount: n("withdraw_amount"),
    withdrawReadyAt: Number(r.withdraw_ready_at ?? 0),
    yieldOwed,
  };
}
