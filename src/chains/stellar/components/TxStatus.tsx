import { toFriendlyError } from "../lib/friendly-error";
import { POOL, explorerTx } from "../lib/pool";

type Action = {
  isError: boolean;
  isSuccess?: boolean;
  error: unknown;
  data: unknown;
};

// The explorer for a result, by its shape (2026-09-25: every result used to become a
// stellar.expert link, including EVM hashes, Solana signatures and plain text).
function explorerUrl(ref: string): string | null {
  if (/^0x[0-9a-fA-F]{64}$/.test(ref)) return explorerTx(POOL.evm.explorer_tx, ref);
  if (/^[0-9a-fA-F]{64}$/.test(ref)) return explorerTx(POOL.stellar.explorer_tx, ref);
  if (/^[1-9A-HJ-NP-Za-km-z]{64,90}$/.test(ref)) return explorerTx(POOL.solana.explorer_tx, ref);
  return null;
}

// Plain-English only, no exceptions. No raw error text, stack trace, or "technical details"
// toggle is ever shown -- every branch below reduces to one short sentence a non-technical user
// can act on. See lib/friendly-error.ts for where the raw error gets translated.
export function TxStatus({ action }: { action: Action }) {
  if (action.isError) {
    const friendly = toFriendlyError(action.error);
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
            view on explorer
          </a>
        ) : (
          <div>{action.data}</div>
        )}
      </div>
    );
  }
  return null;
}
