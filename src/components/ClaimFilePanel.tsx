import { useState } from "react";
import { useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import { checkClaim, fileClaim } from "../lib/claimApi";
import type { CheckClaimResult, ClaimEntry, FileClaimResult } from "../lib/claimApi";
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
//
// B6, 2026-10-30: "ask first". "Check drains" judges each drain on its own and files nothing; the
// user then files only the accepted ones. A drain that wasn't accepted is named, never explained
// (no scanner reason, so a fake claim can't be tuned against it). A filed claim is final.

type Row = { chain: ChainId; txHash: string };

export function ClaimFilePanel() {
  const client = useClient<AppClient>();
  const { address: staker, crossChain } = useStaker(client, "stake");
  const [rows, setRows] = useState<Row[]>([{ chain: EVM_CHAIN_KEY, txHash: "" }]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FileClaimResult | null>(null);
  const [check, setCheck] = useState<CheckClaimResult | null>(null);

  const valid = !!staker && rows.every((r) => r.txHash.trim().length > 0);

  function setRow(i: number, patch: Partial<Row>) {
    setRows((cur) => cur.map((r, j) => (j === i ? { ...r, ...patch } : r)));
    setResult(null);
    setCheck(null);
    setError(null);
  }

  async function runCheck() {
    if (!staker) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setCheck(null);
    try {
      const entries: ClaimEntry[] = rows.map((r) => ({ chain: r.chain, txHash: r.txHash }));
      setCheck(await checkClaim(client, staker, entries));
    } catch (e) {
      setError(toFriendlyError(e).message);
    } finally {
      setRunning(false);
    }
  }

  const accepted: ClaimEntry[] = check
    ? check.results.filter((r) => r.status === "accepted").map((r) => ({ chain: r.chain, txHash: r.txHash }))
    : [];

  async function fileAccepted() {
    if (!staker || !accepted.length) return;
    setRunning(true);
    setError(null);
    try {
      setResult(await fileClaim(client, staker, accepted));
      setCheck(null);
    } catch (e) {
      // Something changed since the check (a scan, a price): nothing was filed.
      setError(`${toFriendlyError(e).message} Nothing was filed. Check the drains again.`);
      setCheck(null);
    } finally {
      setRunning(false);
    }
  }

  function rowLine(i: number, status: string, code: string | null): string {
    const n = rows.length > 1 ? `Drain ${i + 1}` : "This drain";
    if (status === "accepted") return `${n}: accepted.`;
    if (status === "not_accepted") return `${n}: not accepted.`;
    if (status === "fix_input") return `${n}: ${toFriendlyError(new Error(code ?? "")).message}`;
    return `${n}: we couldn't check it right now. Try again shortly.`;
  }

  const OUTCOME_LINE: Record<string, string> = {
    none_accepted: "None of these drains were accepted, so there's nothing to file.",
    fix_input: "Fix the drains marked below, then check again.",
    try_later: "Some drains couldn't be checked right now. Try again shortly, so none of them is left out.",
    incoherent: "These drains don't look like one incident. List only drains from the same incident.",
    refused: "This claim can't be filed.",
  };

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

      <button className="primary-action" disabled={!valid || running} onClick={runCheck}>
        {running && !check ? "Checking the drains..." : "Check drains"}
      </button>

      {check ? (
        <div style={{ marginTop: 12 }}>
          {check.outcome === "ready" ? (
            <div className="ramp-disclosure">
              {accepted.length === check.results.length
                ? `All ${accepted.length} accepted.`
                : `${accepted.length} of ${check.results.length} accepted. The others won't be part of this claim.`}{" "}
              Once filed, a claim is final: drains can't be added or changed later.
            </div>
          ) : (
            <div className="tx-status error">
              <div>{OUTCOME_LINE[check.outcome] ?? OUTCOME_LINE.refused}</div>
            </div>
          )}
          {(check.outcome !== "refused" && check.results.length > 1) || check.outcome === "fix_input" ? (
            <ul style={{ margin: "8px 0", paddingLeft: 18 }}>
              {check.results.map((r, i) => (
                <li key={i}>{rowLine(i, r.status, r.code)}</li>
              ))}
            </ul>
          ) : null}
          {check.outcome === "ready" ? (
            <button className="primary-action" disabled={running} onClick={fileAccepted}>
              {running ? "Filing..." : accepted.length === check.results.length ? "File claim" : `File claim for these ${accepted.length}`}
            </button>
          ) : null}
        </div>
      ) : null}

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
