import type { ComponentType } from "react";
import type { FriendlyError } from "../lib/friendly-error";

export type TxAction = {
  isError: boolean;
  isSuccess?: boolean;
  error: unknown;
  data: unknown;
};

/** The result line under every pool action, ONE for every pool (CIE H3, B42). Each pool binds its own error
 *  table and explorer: `export const TxStatus = makeTxStatus(toFriendlyError, explorerUrl)`.
 *
 *  Plain English only, no exceptions: no raw error text, stack trace or "technical details" toggle is ever
 *  shown. The raw error goes to the console only, for support. */
export function makeTxStatus(
  toFriendly: (error: unknown) => FriendlyError,
  explorerUrl: (ref: string) => string | null,
): ComponentType<{ action: TxAction }> {
  return function TxStatus({ action }) {
    if (action.isError) {
      const friendly = toFriendly(action.error);
      if (friendly.cancelled) return null;
      console.warn("[safu]", friendly.raw);
      return (
        <div className="tx-status error">
          <div>{friendly.message}</div>
          {friendly.action ? <div className="tx-status-hint">{friendly.action}</div> : null}
        </div>
      );
    }
    if (action.data && typeof action.data === "string") {
      const url = explorerUrl(action.data);
      return (
        <div className="tx-status">
          {url ? (
            <a href={url} target="_blank" rel="noreferrer">
              Done · view on explorer
            </a>
          ) : (
            <div>{action.data}</div>
          )}
        </div>
      );
    }
    return null;
  };
}
