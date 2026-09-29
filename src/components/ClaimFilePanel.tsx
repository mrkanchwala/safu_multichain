import { useState } from "react";
import { useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import { fileClaim } from "../lib/claimApi";
import type { ClaimEntry, FileClaimResult } from "../lib/claimApi";
import { CHAINS, EVM_CHAIN_KEY } from "../lib/network";

const MAX_DRAINS = 20; // MAX_TXIDS_PER_CLAIM (SAFU3.0 safu/config.py), enforced again by the backend
import type { ChainId } from "../lib/network";
import { useStaker } from "../lib/useStaker";
import { toFriendlyError } from "../lib/friendly-error";

// File a claim tab. v3, 2026-09-24: ONE claim per stake, bundling up to 20 drain transactions across
// the covered wallets, any chain mix (locked rule, key-decisions.md). The TOTAL payout is capped at
// the stake's tier ceiling (e.g. 5x for tier C) however many drains are listed. The backend auto-matches each drain to one of the staker's
// covered wallets, measures the loss, the hack time and the tier from the chains themselves, and the
// oracle signs + submits. Nothing about the payout is typed in here (no tier, no amount) -- see
// backend/app.py file_claim. Signed by whichever wallet is connected: a Stellar staker directly, an
// EVM/Solana staker's own wallet for its safu-account.

type Row = { chain: ChainId; txHash: string };

export function ClaimFilePanel() {
  const client = useClient<AppClient>();
  const { address: staker, crossChain } = useStaker(client, "stake");
  const [rows, setRows] = useState<Row[]>([{ chain: EVM_CHAIN_KEY, txHash: "" }]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FileClaimResult | null>(null);

  const valid = !!staker && rows.every((r) => r.txHash.trim().length > 0);

  function setRow(i: number, patch: Partial<Row>) {
    setRows((cur) => cur.map((r, j) => (j === i ? { ...r, ...patch } : r)));
    setResult(null);
    setError(null);
  }

  async function submit() {
    if (!staker) return;
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      const entries: ClaimEntry[] = rows.map((r) => ({ chain: r.chain, txHash: r.txHash }));
      setResult(await fileClaim(client, staker, entries));
    } catch (e) {
      setError(toFriendlyError(e).message);
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="field-group" style={{ marginBottom: 20 }}>
      {!client.address ? (
        <div className="ramp-disclosure" style={{ marginBottom: 12 }}>
          Connect the wallet you staked with, since the claim is filed for that stake.
        </div>
      ) : crossChain && staker ? (
        <div className="ramp-disclosure" style={{ marginBottom: 12 }}>
          Filing for your safu-account {staker.slice(0, 6)}...{staker.slice(-4)}. Your wallet signs the request.
        </div>
      ) : null}

      {rows.map((r, i) => (
        <div key={i} className="field-group">
          <div className="field-label">
            <span>Drain {rows.length > 1 ? i + 1 : ""}: chain</span>
            {rows.length > 1 ? (
              <button className="details-link" onClick={() => setRows((cur) => cur.filter((_, j) => j !== i))}>remove</button>
            ) : <span>&nbsp;</span>}
          </div>
          <div className="ramp-toggle" style={{ marginBottom: 8 }}>
            {CHAINS.map((c) => (
              <button
                key={c.id}
                className={`ramp-toggle-btn${r.chain === c.id ? " active" : ""}`}
                onClick={() => setRow(i, { chain: c.id })}
              >
                {c.label}
              </button>
            ))}
          </div>
          <div className="field-input">
            <input placeholder="Drain transaction hash" value={r.txHash} onChange={(e) => setRow(i, { txHash: e.target.value })} />
          </div>
        </div>
      ))}

      {rows.length < MAX_DRAINS ? (
        <button className="secondary-action" style={{ width: "100%", marginBottom: 12 }}
          onClick={() => setRows((cur) => [...cur, { chain: cur[cur.length - 1].chain, txHash: "" }])}>
          + Add another drain (same incident) · {rows.length}/{MAX_DRAINS}
        </button>
      ) : null}

      <div className="ramp-disclosure" style={{ marginBottom: 12 }}>
        One claim per stake. List every drain from the same incident (up to {MAX_DRAINS}, any chains):
        each is matched to one of your covered wallets, and the loss, hack time and tier are read from
        the chains, never typed in. The total payout never exceeds your tier's ceiling.
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
            Filed and submitted on-chain. Open "Collect payout" to approve it and collect.
          </div>
          <div className="side-stat">
            <div className="k">Entitlement</div>
            <div className="v">{(result.entitlementUsdc / 10_000_000).toFixed(2)} USDC</div>
          </div>
          <div className="side-stat">
            <div className="k">Tier</div>
            <div className="v">{result.tier}</div>
          </div>
          <div className="side-stat">
            <div className="k">Claim id</div>
            <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 12 }}>{result.claimIdHex}</div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
