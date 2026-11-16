import { toFriendlyError } from "../lib/friendly-error";
import { explorerTx } from "../lib/pool";

type Action = {
  isError: boolean;
  isSuccess?: boolean;
  error: unknown;
  data: unknown;
};

const SOLANA_SIG = /^[1-9A-HJ-NP-Za-km-z]{64,90}$/;

// Plain-English only, no exceptions. No raw error text, stack trace, or "technical details"
// toggle is ever shown. See lib/friendly-error.ts for where the raw error gets translated.
export function TxStatus({ action }: { action: Action }) {
  if (action.isError) {
    const friendly = toFriendlyError(action.error);
    if (friendly.cancelled) return null;
    // The raw error goes to the console only, for support. The page shows the sentence.
    console.warn("[safu]", friendly.raw);
    return (
      <div className="tx-status error">
        <div>{friendly.message}</div>
        {friendly.action ? <div className="tx-status-hint">{friendly.action}</div> : null}
      </div>
    );
  }
  if (action.data && typeof action.data === "string") {
    return (
      <div className="tx-status">
        {SOLANA_SIG.test(action.data) ? (
          <a href={explorerTx(action.data)} target="_blank" rel="noreferrer">
            Done · view on explorer
          </a>
        ) : (
          <div>{action.data}</div>
        )}
      </div>
    );
  }
  return null;
}
