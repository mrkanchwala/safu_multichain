/**
 * Turns a raw wallet/backend/contract error into one plain sentence a user can act on: what
 * happened, whether their money is safe, and what to do next.
 *
 * Rewritten 2026-09-18 (dropped lending contract) and again 2026-09-25 after a live
 * test: relayed actions, contract errors and the app's own crosschain messages all reached users
 * as the generic line or as jargon ("burn", "reverted", "simulation"). Sources covered:
 *   - contract errors: "Error(Contract, #N)" inside simulation/relay details, N from
 *     contracts/protection-pool/src/error.rs (CONTRACT_ERRORS below);
 *   - backend reason codes: backend/relay.py, app.py, claim.py (ACTION_HINTS);
 *   - wallet errors: rejections, missing gas/USDC (PATTERNS);
 *   - the app's own messages, already plain English (OWN_MESSAGES).
 *
 * IMPORTANT: an unmatched error still falls through to a plain-English generic message at the
 * bottom, never to the raw string. A missing entry here is a worse message, not a leaked code.
 */
import { MAX_STAKE_USDC, MIN_STAKE_USDC } from "./network";

const TRY_LATER = "Nothing was lost. Try again in a few minutes.";
const OUR_SIDE = "Something went wrong on our side. Nothing was lost. Try again, and tell the SAFU team if it keeps happening.";
const NOT_CONFIRMED =
  "The network hasn't confirmed this yet. Wait a minute and check your balance before trying again, so it doesn't happen twice.";
const NETWORK_REFUSED = "The network turned this transaction down. Nothing was taken. Try again.";
const PAYOUT_NOT_STARTED = "Your payout hasn't started yet. There's a waiting period after you approve a claim. Try again later.";
const IN_WAITING_PERIOD = "Your claim is still in its 90-day waiting period. Nothing is lost, it pays once that ends.";
const DAILY_LIMIT = "The pool has reached its payout limit for today. Try again tomorrow.";

/** Protection-pool contract errors (error.rs) a user can actually hit. Admin/governance/oracle
 *  errors are left out on purpose: a user can't cause them, so they get OUR_SIDE. Codes 1-9 are
 *  left out too: the safu-account contract reuses those numbers for its own errors. */
const CONTRACT_ERRORS: Record<number, string> = {
  20: "Enter an amount above zero.",
  21: `A stake has to be between $${MIN_STAKE_USDC} and $${MAX_STAKE_USDC}.`,
  22: "You already have a stake in this pool. Withdraw it first if you want to stake a different amount.",
  23: "The pool is full right now, so it can't take a new stake.",
  28: "This stake was used to pay a claim, so there's nothing left to withdraw.",
  29: "You can't withdraw while a claim on this stake is open.",
  30: "This stake is locked for now. Try again later.",
  32: "You don't have an active stake.",
  43: "This stake is on hold. Contact the SAFU team.",
  44: "You already have a claim open on this stake.",
  47: "This loss happened before you staked, so it isn't covered.",
  48: "It's too late to file a claim for this loss.",
  49: "The pool can't pay this right now. Try again later.",
  50: DAILY_LIMIT,
  51: "The pool has handled its limit of claims for today. Try again tomorrow.",
  52: "This claim has already been filed.",
  53: "We couldn't find that claim.",
  55: IN_WAITING_PERIOD,
  56: "This claim isn't ready to approve yet.",
  57: "The time to approve this claim has run out.",
  59: "This payout hasn't started yet.",
  60: "This payout has already been paid in full.",
  62: PAYOUT_NOT_STARTED,
  63: "Nothing new to collect yet. The payout builds up over time, so try again a bit later.",
  64: DAILY_LIMIT,
  68: "This payout has already been paid in full.",
  69: "This wallet already has a different claim open.",
  73: "This request took too long to go through. Try again.",
  80: "The pool can't pay this right now. Try again later.",
  93: "The pool is paused right now. Try again later.",
  94: "This claim is already waiting in line.",
  96: IN_WAITING_PERIOD,
  105: "You haven't backed the pool yet.",
  106: "You have no new backing waiting to mature.",
  107: "This backing hasn't finished maturing yet.",
  108: "You already asked to withdraw. Complete or cancel that request first.",
  109: "There's no withdrawal request to complete.",
  110: "The notice period isn't over yet.",
  111: "That backing is holding up claims right now and can't be withdrawn yet.",
  112: "That's more than you have backed.",
  113: "You can't withdraw while a claim on this stake is waiting in line.",
  114: "This wallet has an approved claim, so it can't stake again right now.",
};

/** Backend reason codes and fixed phrases, matched as substrings. */
const ACTION_HINTS: Record<string, string> = {
  // Claim filing (claim.py / liquidation_claim.py / app.py)
  ASSET_CHAIN_REJECTED: "This asset isn't covered on that chain yet.",
  WALLET_NOT_COVERED:
    "That wallet isn't one of your covered wallets, or wasn't added before this happened. Covered wallets have to be added before anything happens to them.",
  SCAN_FAILED: "We couldn't check that transaction right now. Try again in a moment.",
  SCAN_DEGRADED: "We couldn't check this properly. Try again shortly, and tell the SAFU team if it keeps happening.",
  VERDICT_NOT_ELIGIBLE: "This transaction doesn't look like a covered loss.",
  LOSS_UNAVAILABLE: "We couldn't work out how much was lost in that transaction.",
  TXID_MALFORMED: "That transaction ID doesn't look right for this chain. Check it and try again.",
  PRICE_UNAVAILABLE: "We couldn't get a reliable price right now. Try again shortly.",
  LIQUIDATION_WAS_FAIR: "This liquidation was priced fairly by the market, so it isn't covered.",
  WAIT_NOT_OVER: "A liquidation can be checked one hour after it happened. Try again then.",
  LENDER_NOT_LISTED: "This lending market isn't one we can check yet.",
  ASSET_NOT_COVERED: "The collateral taken isn't a covered asset.",
  SELF_LIQUIDATION: "This liquidation was carried out by one of your own wallets, so nothing was lost.",
  HACK_TIME_UNAVAILABLE: "We couldn't read when this transaction happened. Check the transaction ID and try again.",
  TIER_UNASSESSABLE: "We couldn't check this wallet's history right now. Try again shortly.",
  WALLET_INELIGIBLE: "This wallet can't be covered.",
  CHAIN_UNSUPPORTED: "That chain isn't supported yet.",
  // WalletConnect 5100: the phone wallet refused the network (usually a test network it has off).
  // Must stay above "Unsupported chain", which is a substring of it.
  "Unsupported chains":
    "Your wallet doesn't support this test network. Turn on test networks in its settings, or use a browser wallet.",
  "Unsupported chain": "That chain isn't supported yet.",
  "Signature expired": "The request took too long, or your device's clock is off. Try again.",
  "Signature does not match": "That signature doesn't match your staking wallet. Connect the wallet you staked with.",
  "owner_chain": OUR_SIDE,
  // Covered wallets (registry)
  TooManyWallets: "You've already added the most wallets you can cover. Remove one before adding another.",
  WalletTaken: "This wallet is already covered by someone else.",
  "already covered by another staker": "This wallet is already covered by someone else.",
  "one of your covered wallets": "That address is one of your covered wallets, so choose a different one.",
  NoActiveStake: "Stake first, then add the wallets you want covered.",
  // Relayer (relay.py)
  WALLET_SIGNATURE_FORMAT: "This wallet can't sign SAFU requests yet. For Solana, use Phantom or Solflare.",
  BAD_SIGNATURE: "Your wallet's signature wasn't accepted. Try again, or use a different wallet.",
  "Error(Auth": "Your wallet's signature wasn't accepted. Try again, or use a different wallet.",
  UNKNOWN_OR_EXPIRED: "This took too long to sign. Try again.",
  RATE_LIMITED: "Too many actions in a short time. Wait a minute and try again.",
  RELAY_BUSY: `A lot of deposits are in progress. ${TRY_LATER}`,
  BURN_ALREADY_QUEUED: "This deposit is already being processed.",
  DEPOSIT_: "Your USDC crossed the bridge but couldn't be added yet. Your money is safe and we'll keep trying. Check back in a few minutes.",
  ATTESTATION_UNAVAILABLE: "Circle's bridge isn't responding right now. Your money is safe. Try again in a few minutes.",
  CHAIN_UNAVAILABLE: `The network isn't responding right now. ${TRY_LATER}`,
  STAKE_READ_FAILED: `The network isn't responding right now. ${TRY_LATER}`,
  BROADCAST_FAILED: `The network isn't responding right now. ${TRY_LATER}`,
  SEND_FAILED: NETWORK_REFUSED,
  TX_FAILED: NETWORK_REFUSED,
  NOT_CONFIRMED,
  "not configured": `The service is being updated. ${TRY_LATER}`,
  "not deployed": `The service is being updated. ${TRY_LATER}`,
  // Direct Stellar calls (soroban.ts)
  "submission rejected": NETWORK_REFUSED,
  "rejected on chain": NETWORK_REFUSED,
  "still unconfirmed": NOT_CONFIRMED,
  // Anything else the relayer or claim API refuses is a bug on our side, not something to fix.
  NOT_A_SAFU_ACCOUNT: OUR_SIDE,
  REFUSED: OUR_SIDE,
  UNKNOWN_ACTION: OUR_SIDE,
  BAD_ARGS: OUR_SIDE,
  NOT_A_HOME_BURN: OUR_SIDE,
  BAD_BURN_TX: OUR_SIDE,
  BAD_TX_HASH: OUR_SIDE,
  SIMULATION_FAILED: OUR_SIDE,
  PREPARE_FAILED: OUR_SIDE,
};

/** Wallet and network errors, which have no fixed code. Checked after the codes above. */
const PATTERNS: [RegExp, string][] = [
  [/user rejected|reject.*request|declined|denied/i, "Request was declined in the wallet."],
  [/insufficient.*(xlm|native)/i, "Not enough XLM in the wallet to cover the network fee."],
  [/insufficient funds for (gas|intrinsic)|gas required exceeds/i, "Not enough ETH in the wallet to cover the network fee."],
  [/insufficient lamports|no record of a prior credit/i, "Not enough SOL in the wallet to cover the network fee."],
  [/exceeds balance|insufficient (funds|balance)/i, "You don't have enough USDC in this wallet."],
  [/failed to fetch|networkerror|load failed|request failed \(5\d\d\)/i, "We couldn't reach SAFU. Check your connection and try again."],
];

// Messages this app throws itself, already written for a user: shown as-is.
const OWN_MESSAGES = [
  "Connect a",
  "Connect the",
  "No EVM wallet found",
  "No Solana wallet found",
  "Wrong network",
  "The wallet returned",
  "This wallet",
  "Stake first",
  "Circle",
  "Your ",
  "Couldn't",
  "You don't",
  "Stopped.",
  // Backend messages written as sentences (app.py)
  "Too many transactions",
  "The same transaction",
];

export type FriendlyError = {
  message: string;
  action?: string;
  raw: string;
  /** The user closed the wallet window themselves: not an error, show nothing. */
  cancelled?: boolean;
};

// Wallet SDKs don't always throw Errors: stellar-wallets-kit rejects with a
// plain `{ code: -1, message: "The user closed the modal." }` (esm/sdk/kit.js).
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

  if (OWN_MESSAGES.some((m) => raw.startsWith(m))) {
    return { message: raw, raw };
  }

  const contract = /Error\(Contract, #(\d+)\)/.exec(raw);
  if (contract && CONTRACT_ERRORS[Number(contract[1])]) {
    return { message: CONTRACT_ERRORS[Number(contract[1])], raw };
  }

  for (const [reason, message] of Object.entries(ACTION_HINTS)) {
    if (raw.includes(reason)) {
      return { message, raw };
    }
  }

  for (const [pattern, message] of PATTERNS) {
    if (pattern.test(raw)) {
      return { message, raw };
    }
  }

  return {
    message: "This didn't go through. Try again in a moment, and tell the SAFU team if it keeps happening.",
    raw,
  };
}
