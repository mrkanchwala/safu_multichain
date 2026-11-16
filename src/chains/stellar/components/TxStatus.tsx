import { makeTxStatus } from "../../../shell/components/TxStatus";
import { toFriendlyError } from "../lib/friendly-error";
import { POOL, explorerTx } from "../lib/pool";

// The explorer for a result, by its shape (2026-09-25: every result used to become a
// stellar.expert link, including EVM hashes, Solana signatures and plain text).
function explorerUrl(ref: string): string | null {
  if (/^0x[0-9a-fA-F]{64}$/.test(ref)) return explorerTx(POOL.evm.explorer_tx, ref);
  if (/^[0-9a-fA-F]{64}$/.test(ref)) return explorerTx(POOL.stellar.explorer_tx, ref);
  if (/^[1-9A-HJ-NP-Za-km-z]{64,90}$/.test(ref)) return explorerTx(POOL.solana.explorer_tx, ref);
  return null;
}

/** The shared result line (shell/components/TxStatus.tsx) with this pool's errors and explorers. */
export const TxStatus = makeTxStatus(toFriendlyError, explorerUrl);
