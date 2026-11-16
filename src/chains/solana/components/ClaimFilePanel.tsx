import { useState } from "react";
import { useClient } from "../lib/client";
import { fileClaim } from "../lib/claimApi";
import type { FileClaimResult } from "../lib/claimApi";
import { toFriendlyError } from "../lib/friendly-error";
import { fmtSol } from "../lib/reads";
import { explorerTx, num } from "../lib/pool";

// File a claim tab. ONE claim per stake, bundling up to MAX_TXIDS_PER_CLAIM drain transactions from the
// covered wallet. The backend matches each drain to a covered wallet, measures the loss, the hack time and the
// tier from the chain, and the oracle submits the claim. Nothing about the payout is typed in here.
const MAX_DRAINS = Number(num("MAX_TXIDS_PER_CLAIM")); // IDL; the claim API enforces the same constant

export function ClaimFilePanel() {
  const client = useClient();
  const staker = client.address;
  const [rows, setRows] = useState<string[]>([""]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FileClaimResult | null>(null);

  const valid = !!staker && rows.every((r) => r.trim().length > 0);

  function setRow(i: number, v: string) {
    setRows((cur) => cur.map((r, j) => (j === i ? v : r)));
    setResult(null);
    setError(null);
  }

  async function submit() {
    if (!staker) return;
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      setResult(await fileClaim(client, staker, rows));
    } catch (e) {
      setError(toFriendlyError(e).message);
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="field-group" style={{ marginBottom: 20 }}>
      {!staker ? (
        <div className="ramp-disclosure" style={{ marginBottom: 12 }}>
          Connect the wallet you staked with, since the claim is filed for that stake.
        </div>
      ) : null}

      {rows.map((r, i) => (
        <div key={i} className="field-group">
          <div className="field-label">
            <span>Drain transaction {rows.length > 1 ? i + 1 : ""}</span>
            {rows.length > 1 ? (
              <button className="details-link" onClick={() => setRows((cur) => cur.filter((_, j) => j !== i))}>remove</button>
            ) : <span>&nbsp;</span>}
          </div>
          <div className="field-input">
            <input placeholder="Solana signature or Ethereum tx hash (0x...)" value={r} onChange={(e) => setRow(i, e.target.value)} aria-label={`Drain transaction ${i + 1}`} />
          </div>
        </div>
      ))}

      {rows.length < MAX_DRAINS ? (
        <button className="secondary-action" style={{ width: "100%", marginBottom: 12 }} onClick={() => setRows((cur) => [...cur, ""])}>
          + Add another drain (same incident) · {rows.length}/{MAX_DRAINS}
        </button>
      ) : null}

      <div className="ramp-disclosure" style={{ marginBottom: 12 }}>
        One claim per stake. List every drain from the same incident (up to {MAX_DRAINS}): each is matched to
        your covered wallets, and the loss, the time it happened and your tier are read from the chain,
        never typed in. The total payout never goes above your tier's ceiling.
      </div>

      <button className="primary-action" disabled={!valid || running} onClick={submit}>
        {running ? "Checking the drains..." : "File claim"}
      </button>

      {error ? (
        <div className="tx-status error" style={{ marginTop: 12 }}>
          <div>{error}</div>
        </div>
      ) : null}

      {result ? (
        <div style={{ marginTop: 12 }}>
          <div className="ramp-disclosure">
            Filed on-chain. Open "Collect payout" for the next step.{" "}
            <a href={explorerTx(result.tx_signature)} target="_blank" rel="noreferrer">View on explorer</a>
          </div>
          <div className="side-stat">
            <div className="k">Payout</div>
            <div className="v">{fmtSol(BigInt(result.entitlement_lamports))} SOL</div>
          </div>
          <div className="side-stat">
            <div className="k">Tier</div>
            <div className="v">{result.tier}</div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
