import type { AppClient } from "./client";
import { addr, bytesn32, i128, invokeContract, isLive, poolId } from "./soroban";
import { USDC_DECIMALS } from "./network";

// Every action routes through one gate: `isLive()`. Until POOL_CONTRACT_ID in
// network.ts is filled in after deployment, each call returns an obviously-fake
// signature after a short delay and touches no network; once it is set, the
// same call goes to the real contract. Panel components never change, and
// there is no separate build to merge on the night.
//
// Pruned 2026-09-18: depositAndBorrow / repay / withdrawCollateral / supply /
// withdrawSupply / backerDeposit / backerRequestWithdraw / backerFinalizeWithdraw
// / buyInventory were the old vault+backstop money-market calls -- dead with
// the dropped money market. `stake`/`withdrawStake` (A8), `setBeneficiary` and
// `pullClaimPayout` (Claims tab) are what remains/is real for this build.

const toRaw = (amountUsdc: string): bigint =>
  BigInt(Math.round(Number(amountUsdc) * 10 ** USDC_DECIMALS));

function fakeSignature(): Promise<string> {
  return new Promise((resolve) =>
    setTimeout(() => resolve("SHELL_STUB_" + Math.random().toString(36).slice(2, 10)), 400),
  );
}

function requireAddress(client: AppClient): string {
  if (!client.address || client.kind !== "stellar") {
    throw new Error("Connect a Stellar wallet first.");
  }
  return client.address;
}

/**
 * Stake USDC into the pool. Real signature: `stake(staker, amount, beneficiary)`
 * (`lib.rs:174`). Beneficiary defaults to the staker's own address if not
 * given -- the contract refuses `beneficiary == staker` outright
 * (`BeneficiaryIsStaker`, `stake.rs:153`), so passing the connected wallet's
 * own address for both would fail on-chain; callers wanting the default
 * payout-to-self behavior should pass a genuinely different address (a
 * cold wallet, a multisig) or the UI should require an explicit choice.
 */
export async function stake(
  client: AppClient,
  _owner: unknown,
  amountUsdc: string,
  beneficiary: string,
): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(
    poolId(),
    "stake",
    [addr(who), i128(toRaw(amountUsdc)), addr(beneficiary)],
    who,
  );
}

/**
 * Withdraw a stake -- principal plus accrued yield, computed on-chain by
 * `get_withdrawable_amount`'s index ratio (A7). Real signature:
 * `withdraw(staker, beneficiary)` (`lib.rs:183`) -- takes no amount; the
 * contract pays out the staker's full current balance, not a partial one.
 */
export async function withdrawStake(
  client: AppClient,
  _owner: unknown,
  beneficiary: string,
): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "withdraw", [addr(who), addr(beneficiary)], who);
}

/**
 * r3: take the stake's yield without unstaking. Real signature:
 * `claim_yield(staker, beneficiary)` -- paid to the beneficiary, which must
 * match the stake's (hash-checked on-chain, same as `withdraw`).
 */
export async function claimStakerYield(client: AppClient, beneficiary: string): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "claim_yield", [addr(who), addr(beneficiary)], who);
}

/**
 * Redirect a stake's payout to a different beneficiary.
 *
 * Renamed from `setPayoutAddress` 2026-09-18 -- the old name called a method
 * that does not exist on this pool. The real function is `set_beneficiary`
 * (`stake.rs:197`), which takes (staker, new_beneficiary) and needs no
 * separate caller argument -- `staker.require_auth()` is the caller.
 */
export async function setBeneficiary(
  client: AppClient,
  _owner: unknown,
  newBeneficiary: string,
): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "set_beneficiary", [addr(who), addr(newBeneficiary)], who);
}

/**
 * Approve a claim that has cleared the 90-day gate -- forfeits principal
 * and starts the 7-day cooldown before streaming begins. Real signature:
 * `approve_claim(claim_id: BytesN<32>)` (`lib.rs:279`), no separate caller
 * argument -- `claim.wallet.require_auth()` inside the contract is the
 * staker's own signature on this call, not a parameter.
 *
 * This is a REQUIRED step, not optional: `AwaitingApproval` claims sit
 * inert without it, and lapse to `Expired` after the 100-day window
 * (`APPROVE_WINDOW_LEDGERS`) with the stake never forfeited and nothing
 * paid. Missing this button would leave the real claim lifecycle with no
 * way to actually complete from the UI.
 */
export async function approveClaim(client: AppClient, _owner: unknown, claimIdHex: string): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "approve_claim", [bytesn32(claimIdHex)], who);
}

/**
 * Pull a vested claim payout.
 *
 * FIXED 2026-09-19 (A9). Real signature:
 * `claim_stream(claim_id: BytesN<32>, beneficiary: Address)` (`claim.rs:1035`)
 * -- a 32-byte claim ID, not the old vault's (claimAddress, claimSeq)
 * numeric-sequence shape this called before the pivot. `claimIdHex` must be
 * the SAME value `backend/submit.py`'s `SubmitClaimCall.claim_id_hex`
 * produces (`compute_claim_id(wallet, tx_hash)` on-chain, `claim.rs:238`) --
 * there is no separate frontend derivation of it, by design, so the two
 * sides cannot silently disagree on what a claim's id is.
 */
export async function pullClaimPayout(
  client: AppClient,
  _payer: unknown,
  claimIdHex: string,
): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "claim_stream", [bytesn32(claimIdHex), addr(who)], who);
}

// --- Backers (Back the pool tab, 2026-09-24) --------------------------------------------------
// Real signatures (protection-pool lib.rs:253-275). Backing adds capacity; it is never a stake and
// carries no covered wallets. `mature_backing` takes no auth: anyone may move matured money into
// capacity, so the backer can press it themselves once the maturity time has passed.

export async function backPool(client: AppClient, amountUsdc: string): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "back", [addr(who), i128(toRaw(amountUsdc))], who);
}

export async function matureBacking(client: AppClient): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "mature_backing", [addr(who)], who);
}

export async function requestBackerWithdrawal(client: AppClient, amountRaw: bigint): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "request_backer_withdrawal", [addr(who), i128(amountRaw)], who);
}

export async function cancelBackerWithdrawal(client: AppClient): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "cancel_backer_withdrawal", [addr(who)], who);
}

export async function completeBackerWithdrawal(client: AppClient): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "complete_backer_withdrawal", [addr(who)], who);
}

/** r3: take backer yield any time, principal untouched. `claim_backer_yield(backer)`. */
export async function claimBackerYield(client: AppClient): Promise<string> {
  if (!isLive()) return fakeSignature();
  const who = requireAddress(client);
  return invokeContract(poolId(), "claim_backer_yield", [addr(who)], who);
}
