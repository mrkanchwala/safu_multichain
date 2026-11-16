/**
 * Turns a raw wallet/backend/contract error into one plain sentence a user can act on: what happened,
 * whether their money is safe, and what to do next. ONE matcher for every pool (CIE H3, B42); each pool
 * passes its own table (chains/<pool>/lib/friendly-error.ts) for the codes only it can raise.
 *
 * Order: wallet window closed → the app's own sentences → the pool's special codes (contract numbers,
 * program error names) → reason codes → wallet/network patterns → a plain generic sentence.
 *
 * IMPORTANT: an unmatched error still falls through to the generic sentence, never to the raw string.
 * A missing entry is a worse message, not a leaked code.
 */
import { REQUEST_TIMEOUT } from "./http";

export const TRY_LATER = "Nothing was lost. Try again in a few minutes.";
export const OUR_SIDE =
  "Something went wrong on our side. Nothing was lost. Try again, and tell the SAFU team if it keeps happening.";
export const DAILY_LIMIT = "The pool has reached its payout limit for today. Try again tomorrow.";
const GENERIC = "This didn't go through. Try again in a moment, and tell the SAFU team if it keeps happening.";

/** Reason codes every pool shares (the site's own API client). */
const SHARED_HINTS: Record<string, string> = {
  [REQUEST_TIMEOUT]:
    "SAFU took too long to answer. Check back in a minute before trying again, so nothing happens twice.",
};

export type FriendlyError = {
  message: string;
  action?: string;
  raw: string;
  /** The user closed the wallet window themselves: not an error, show nothing. */
  cancelled?: boolean;
};

export type ErrorTable = {
  /** Messages the pool's own code throws, already written for a user: shown as-is (prefix match). */
  own: string[];
  /** The pool's own codes (contract error numbers, program error names); undefined = not one of them. */
  special?: (raw: string) => string | undefined;
  /** Backend reason codes and fixed phrases, matched as substrings, in order. */
  hints: Record<string, string>;
  /** Wallet and network errors with no fixed code, checked last. */
  patterns: [RegExp, string][];
};

// Wallet SDKs don't always throw Errors: stellar-wallets-kit rejects with a plain
// `{ code: -1, message: "The user closed the modal." }`.
function rawText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && typeof (error as { message?: unknown }).message === "string") {
    return (error as { message: string }).message;
  }
  return String(error);
}

export function makeFriendlyError(table: ErrorTable): (error: unknown) => FriendlyError {
  const hints = { ...SHARED_HINTS, ...table.hints };
  return (error) => {
    const raw = rawText(error);
    if (/closed the modal|user closed|modal closed|cancell?ed by (the )?user/i.test(raw)) {
      return { message: "Wallet window closed.", raw, cancelled: true };
    }
    if (table.own.some((m) => raw.startsWith(m))) return { message: raw, raw };
    const special = table.special?.(raw);
    if (special) return { message: special, raw };
    for (const [reason, message] of Object.entries(hints)) {
      if (raw.includes(reason)) return { message, raw };
    }
    for (const [pattern, message] of table.patterns) {
      if (pattern.test(raw)) return { message, raw };
    }
    return { message: GENERIC, raw };
  };
}
