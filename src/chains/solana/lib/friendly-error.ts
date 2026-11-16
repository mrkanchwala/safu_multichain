/**
 * Turns a raw wallet/backend/program error into one plain sentence a user can act on: what happened,
 * whether their money is safe, and what to do next. From the multichain site's friendly-error.ts,
 * rewritten for the Solana pool:
 *   - program errors: `PROGRAM_ERROR:<name>` from lib/program.ts, names from the IDL (PROGRAM_ERRORS);
 *   - backend reason codes: backend/colosseum_app.py (ACTION_HINTS);
 *   - wallet errors: rejections, missing SOL (PATTERNS);
 *   - the app's own messages, already plain English (OWN_MESSAGES).
 *
 * IMPORTANT: an unmatched error still falls through to a plain-English generic message at the
 * bottom, never to the raw string. A missing entry here is a worse message, not a leaked code.
 */

const TRY_LATER = "Nothing was lost. Try again in a few minutes.";
const OUR_SIDE = "Something went wrong on our side. Nothing was lost. Try again, and tell the SAFU team if it keeps happening.";
const NETWORK_BUSY = `The network isn't responding right now. ${TRY_LATER}`;
const DAILY_LIMIT = "The pool has reached its payout limit for today. Try again tomorrow.";
const NO_LIQUIDITY = "The pool can't pay this out right now. Nothing was lost. Try again later.";

/** Program errors (IDL `errors`) a user can actually hit. Admin/oracle ones are left out on purpose:
 *  a user can't cause them, so they get OUR_SIDE. */
const PROGRAM_ERRORS: Record<string, string> = {
  Paused: "The pool is paused right now. Try again later.",
  StakeOutOfRange: "That amount is outside the pool's stake limits shown above the box.",
  PoolCapExceeded: "The pool is full right now, so it can't take this stake.",
  AlreadyStaked: "This wallet already has a stake. Withdraw it first to stake a different amount.",
  AddressHasApprovedClaim: "This wallet was paid out by a claim, so it can't stake again. Use a new wallet.",
  NoActiveStake: "You don't have an active stake.",
  StakeForfeited: "This stake was used to pay a claim, so there's nothing left to withdraw.",
  ClaimActive: "You can't withdraw while a claim on this stake is open.",
  ClaimQueuedForStake: "You can't withdraw while a claim on this stake is waiting in line.",
  PenaltyLockActive: "This stake is locked for a short time after a cancelled claim. Try again later.",
  InsufficientLiquidity: NO_LIQUIDITY,
  AmountNotPositive: "Enter an amount above zero.",
  NoPendingBacking: "You have no new backing waiting to mature.",
  BackingNotMature: "This backing hasn't finished maturing yet.",
  BackerWithdrawalPending: "You already asked to withdraw. Complete or cancel that request first.",
  BackerAmountExceedsBalance: "That's more than you have backed.",
  NoBackerWithdrawal: "There's no withdrawal request to complete.",
  BackerNoticeNotPassed: "The notice period isn't over yet.",
  CapitalNotFree: "Open claims need that money right now, so it stays in the pool until they're paid. Nothing is lost. Take out a smaller amount, or try again later.",
  StakeBelowMinimum: "What stays in your stake would be below the minimum. Take it all out instead, or take out less.",
  AmountExceedsStake: "That's more than your stake.",
  BeneficiaryIsPoolAccount: OUR_SIDE,
  StakeSuspended: "Payouts on this stake are on hold. Contact the SAFU team.",
  NoSuchQueuedClaim: "This claim isn't waiting in line anymore. Refresh the page.",
  QueueReleaseNotYetEligible: "The pool doesn't have room for this claim yet. It stays in line; try again later.",
  ClaimNotPending: "This claim has already moved past its waiting period.",
  TimeGateNotMet: "This claim's waiting period isn't over yet.",
  ClaimNotAwaitingApproval: "This claim isn't ready to approve.",
  ApprovalWindowExpired: "The time to approve this claim has run out.",
  ClaimStakeMismatch: "This claim belongs to a different stake. Connect the wallet you staked with.",
  ClaimNotActive: "This payout hasn't started yet.",
  ClaimFullyStreamed: "This payout has already been paid in full.",
  CooldownNotPassed: "Your payout hasn't started yet. There's a short wait after you approve a claim.",
  NothingVested: "Nothing new to collect yet. The payout builds up over time, so try again a bit later.",
  DailyOutflowCapReached: DAILY_LIMIT,
  ClaimAlreadyCompleted: "This payout has already been paid in full.",
  Insolvent: NO_LIQUIDITY,
  UnstakeFeeTooHigh: "Marinade's unstake fee is too high right now. Nothing was lost. Try again later.",
  NothingToClaim: "There's no yield to take yet.",
  WrongBeneficiary: OUR_SIDE,
  WrongMarinadeAccount: OUR_SIDE,
};

/** Backend reason codes and fixed phrases (colosseum_app.py), matched as substrings. */
const ACTION_HINTS: Record<string, string> = {
  // Marinade refused the unstake a payout needed (lib/program.ts explain).
  MARINADE_REFUSED: "Marinade couldn't turn the pool's mSOL back into SOL for this right now. Nothing was lost. Try again later, or try a smaller amount.",
  STAKER_WALLET: "A covered wallet can't be your staking wallet. Stake from a separate wallet and cover the ones that hold your money.",
  WALLET_INVALID: "That isn't a valid address for the chain you picked. Check it and try again.",
  CHAIN_UNSUPPORTED: "Covered wallets can be on Solana or Ethereum only.",
  WALLET_NOT_COVERED:
    "That wallet isn't one of your covered wallets, or wasn't added before this happened. Covered wallets have to be added before anything happens to them.",
  SCAN_FAILED: "We couldn't check that transaction right now. Try again in a moment.",
  VERDICT_NOT_ELIGIBLE: "This transaction doesn't look like a covered loss.",
  LOSS_UNAVAILABLE: "We couldn't work out how much was lost in that transaction.",
  PRICE_UNAVAILABLE: "We couldn't get a reliable price right now. Try again shortly.",
  HACK_TIME_UNAVAILABLE: "We couldn't read when this transaction happened. Check the transaction ID and try again.",
  TIER_UNASSESSABLE: "We couldn't check this wallet's history right now. Try again shortly.",
  WALLET_INELIGIBLE: "This wallet didn't have enough history before the hack to be covered, so this loss can't be paid.",
  "Signature expired": "The request took too long, or your device's clock is off. Try again.",
  "Signature does not match": "That signature doesn't match your staking wallet. Connect the wallet you staked with.",
  TooManyWallets: "You've already added the most wallets you can cover.",
  WalletTaken: "This wallet is already covered by someone else.",
  NoActiveStake: "Stake first, then add the wallets you want covered.",
  StakeForfeited: "This stake already paid a claim.",
  CHAIN_UNAVAILABLE: NETWORK_BUSY,
  STAKE_READ_FAILED: NETWORK_BUSY,
  BROADCAST_FAILED: NETWORK_BUSY,
  "Pool not available": `The pool is being updated. ${TRY_LATER}`,
  "another cluster": OUR_SIDE,
};

/** Wallet and network errors, which have no fixed code. Checked after the codes above. */
const PATTERNS: [RegExp, string][] = [
  [/user rejected|reject.*request|declined|denied/i, "Request was declined in the wallet."],
  [/insufficient lamports|no record of a prior credit|insufficient funds/i, "Not enough SOL in this wallet."],
  [/blockhash not found|block height exceeded/i, "This took too long to sign. Try again."],
  [/failed to fetch|networkerror|load failed/i, "We couldn't reach SAFU. Check your connection and try again."],
  // The API answered, so the connection is fine: a 5xx is a fault on SAFU's side.
  [/request failed \(5\d\d\)/i, OUR_SIDE],
];

// Messages this app throws itself, already written for a user: shown as-is.
const OWN_MESSAGES = [
  "Connect a",
  "Connect the",
  "No Solana wallet found",
  "Wrong network",
  "The wallet returned",
  "This wallet",
  "The transaction",
  "The pool",
  "Stake first",
  "Your ",
  "You don't",
  "Enter ",
  "Too many transactions",
  "The same transaction",
  "No transactions given",
];

export type FriendlyError = {
  message: string;
  action?: string;
  raw: string;
  /** The user closed the wallet window themselves: not an error, show nothing. */
  cancelled?: boolean;
};

// Wallet SDKs don't always throw Errors: some reject with a plain `{ code, message }` object.
function rawText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && typeof (error as { message?: unknown }).message === "string") {
    return (error as { message: string }).message;
  }
  return String(error);
}

export function toFriendlyError(error: unknown): FriendlyError {
  const raw = rawText(error);

  if (/closed the modal|user closed|modal closed|cancell?ed by (the )?user/i.test(raw)) {
    return { message: "Wallet window closed.", raw, cancelled: true };
  }
  if (OWN_MESSAGES.some((m) => raw.startsWith(m))) return { message: raw, raw };

  // The covered wallet fails the history floors (backend tier_common.check_eligible); keep its numbers.
  const txs = /WALLET_INELIGIBLE: covered wallet had (\d+) transactions at the hack; at least (\d+)/.exec(raw);
  if (txs) {
    return {
      message: `This wallet had only ${txs[1]} transactions before the hack. A covered wallet needs at least ${txs[2]}, so this loss can't be paid.`,
      raw,
    };
  }
  const age = /WALLET_INELIGIBLE: covered wallet was (\d+)d old at the hack; at least (\d+) days/.exec(raw);
  if (age) {
    return {
      message: `This wallet was only ${age[1]} days old at the hack. A covered wallet must be at least ${age[2]} days old, so this loss can't be paid.`,
      raw,
    };
  }

  const program = /PROGRAM_ERROR:(\w+)/.exec(raw);
  if (program) return { message: PROGRAM_ERRORS[program[1]] ?? OUR_SIDE, raw };

  for (const [reason, message] of Object.entries(ACTION_HINTS)) {
    if (raw.includes(reason)) return { message, raw };
  }
  for (const [pattern, message] of PATTERNS) {
    if (pattern.test(raw)) return { message, raw };
  }
  return {
    message: "This didn't go through. Try again in a moment, and tell the SAFU team if it keeps happening.",
    raw,
  };
}
