import { makeTxStatus } from "../../../shell/components/TxStatus";
import { toFriendlyError } from "../lib/friendly-error";
import { explorerTx } from "../lib/pool";

const SOLANA_SIG = /^[1-9A-HJ-NP-Za-km-z]{64,90}$/;

/** The shared result line (shell/components/TxStatus.tsx) with this pool's errors and explorer. */
export const TxStatus = makeTxStatus(toFriendlyError, (ref) => (SOLANA_SIG.test(ref) ? explorerTx(ref) : null));
