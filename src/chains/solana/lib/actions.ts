// Every user action, one program instruction each, signed by the connected wallet.
// Payouts (withdraw, yield, claim stream) go to the stake's beneficiary, which this site always sets
// to the staking wallet itself.
import { backerAddress, idlIx, legAccounts, send, stakeAddress } from "./program";
import type { PoolRecord, Signer } from "./program";

async function run(w: Signer, pool: PoolRecord | null, name: string, accounts: Record<string, string>, args?: Record<string, unknown>, leg = false) {
  if (!w.address) throw new Error("Connect a wallet first.");
  if (leg && !pool) throw new Error("The pool isn't available right now. Try again in a minute.");
  const all = leg && pool ? { ...(await legAccounts(pool)), ...accounts } : accounts;
  return send(w, [await idlIx(name, all, args)]);
}

const me = (w: Signer) => w.address as string;

export const stake = (w: Signer, pool: PoolRecord | null, lamports: bigint) =>
  run(w, pool, "stake", { staker: me(w) }, { amount: lamports, beneficiary: me(w) }, true);
/** `lamports` of principal (all of it, or a part that leaves at least the min stake) + all yield. */
export const withdraw = (w: Signer, pool: PoolRecord | null, lamports: bigint) =>
  run(w, pool, "withdraw", { staker: me(w), beneficiary: me(w) }, { amount: lamports }, true);
export const claimYield = (w: Signer, pool: PoolRecord | null) =>
  run(w, pool, "claim_yield", { staker: me(w), beneficiary: me(w) }, {}, true);

export const back = (w: Signer, pool: PoolRecord | null, lamports: bigint) =>
  run(w, pool, "back", { backer: me(w) }, { amount: lamports }, true);
// Permissionless: no backer account in the instruction, so its record is passed in.
export const matureBacking = async (w: Signer, pool: PoolRecord | null) =>
  run(w, pool, "mature_backing", { backer_record: await backerAddress(me(w)) }, {}, true);
export const requestBackerWithdrawal = (w: Signer, pool: PoolRecord | null, lamports: bigint) =>
  run(w, pool, "request_backer_withdrawal", { backer: me(w) }, { amount: lamports }, true);
export const cancelBackerWithdrawal = (w: Signer, pool: PoolRecord | null) =>
  run(w, pool, "cancel_backer_withdrawal", { backer: me(w) });
export const completeBackerWithdrawal = (w: Signer, pool: PoolRecord | null) =>
  run(w, pool, "complete_backer_withdrawal", { backer: me(w) }, {}, true);
export const claimBackerYield = (w: Signer, pool: PoolRecord | null) =>
  run(w, pool, "claim_backer_yield", { backer: me(w) }, {}, true);

// Claim steps. Moving a queued/pending claim forward is permissionless; the connected wallet just pays the fee.
export const releaseQueuedClaim = async (w: Signer, pool: PoolRecord | null, staker: string, claim: string) =>
  run(w, pool, "try_release_queued_claim", { claim, stake_record: await stakeAddress(staker) });
export const unlockPendingClaim = async (w: Signer, pool: PoolRecord | null, staker: string, claim: string) =>
  run(w, pool, "unlock_pending_claim", { claim, stake_record: await stakeAddress(staker) });
export const approveClaim = (w: Signer, pool: PoolRecord | null, claim: string) =>
  run(w, pool, "approve_claim", { staker: me(w), claim });
export const collectClaim = (w: Signer, pool: PoolRecord | null, claim: string, beneficiary: string) =>
  run(w, pool, "claim_stream", { staker: me(w), claim, beneficiary }, {}, true);
