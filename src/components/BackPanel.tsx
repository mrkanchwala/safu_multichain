import { useEffect, useState } from "react";
import { POOL } from "../lib/pool";
import { useAction, useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import {
  backPool,
  cancelBackerWithdrawal,
  claimBackerYield,
  completeBackerWithdrawal,
  matureBacking,
  requestBackerWithdrawal,
} from "../lib/actions";
import { fmtUsdc, readMyBacking, readPoolStats } from "../lib/reads";
import type { MyBacking } from "../lib/reads";
import { TxStatus } from "./TxStatus";
import { MIN_BRIDGEABLE_RAW } from "../lib/network";
import { evmBurnToAdapter, finishOnStellar, relayAction, relayHomeAction, relayMature, solanaBurnToAdapter } from "../lib/crosschain";
import { useStaker } from "../lib/useStaker";
import { useTick } from "../lib/useTick";

// Back the pool tab (2026-09-24): like Stake, without covered wallets. Backing adds
// capacity for claims; it earns no coverage. Stellar wallets act directly. EVM/Solana wallets back
// through the BACK adapter over CCTP (its own safu-account, separate from the staking one; proven
// live by e2e groups BB/BBS); later steps are owner-signed and relayed.

function when(ts: number): string {
  if (!ts) return "--";
  const d = new Date(ts * 1000);
  return d.getTime() <= Date.now() ? "now" : d.toLocaleString();
}

export function BackPanel() {
  const client = useClient<AppClient>();
  const { address: backer, crossChain } = useStaker(client, "back");
  const [progress, setProgress] = useState<string | null>(null);
  const [amount, setAmount] = useState("");
  const [backing, setBacking] = useState<MyBacking | null>(null);
  const [pool, setPool] = useState({ totalStaked: 0n, totalStakers: 0 });
  const [refreshKey, setRefreshKey] = useState(0);
  // Also re-renders, so the buttons below appear when a "ready" time passes.
  const tick = useTick();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const stats = await readPoolStats(client);
      const b = backer ? await readMyBacking(backer) : null;
      if (!cancelled) {
        setPool(stats);
        setBacking(b);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, backer, refreshKey, tick]);

  const done = <T,>(p: Promise<T>) => p.finally(() => setRefreshKey((k) => k + 1));
  const backAction = useAction((signal) => done((async () => {
    if (!crossChain) return backPool(client, amount);
    const kind = client.kind as "evm" | "solana";
    try {
      setProgress(`Burning ${amount} USDC on ${kind === "evm" ? POOL.evm.label : POOL.solana.label}...`);
      const usdc6 = BigInt(Math.round(Number(amount) * 1e6));
      const burn = kind === "evm" ? await evmBurnToAdapter(client, usdc6, "back", setProgress) : await solanaBurnToAdapter(client, usdc6, "back");
      const account = await finishOnStellar("back", kind, burn, setProgress, signal);
      return `Backing added from your safu-account ${account.slice(0, 6)}...`;
    } finally {
      setProgress(null);
    }
  })()));
  const need = () => {
    if (!backer) throw new Error("Connect a wallet first.");
    return backer;
  };
  const matureAction = useAction(() => done(crossChain ? relayMature(need()) : matureBacking(client)));
  const requestAction = useAction(() => done(crossChain
    ? relayAction(client, need(), "request_back_withdrawal", { amount: String(backing?.amount ?? 0n) })
    : requestBackerWithdrawal(client, backing?.amount ?? 0n)));
  const cancelAction = useAction(() => done(crossChain ? relayAction(client, need(), "cancel_back_withdrawal") : cancelBackerWithdrawal(client)));
  const completeAction = useAction(() => done(crossChain
    ? relayHomeAction(client, need(), "complete_back_withdrawal_home", {}, setProgress).finally(() => setProgress(null))
    : completeBackerWithdrawal(client)));
  // r3: take backer yield any time, backing untouched. EVM / Solana: sent home over CCTP.
  const takeYieldAction = useAction(() => done(crossChain
    ? relayHomeAction(client, need(), "claim_backer_yield_home", {}, setProgress).finally(() => setProgress(null))
    : claimBackerYield(client)));

  const now = Date.now() / 1000;
  const canMature = !!backing && backing.pendingAmount > 0n && backing.pendingMaturesAt <= now;
  const canRequest = !!backing && backing.amount > 0n && backing.withdrawAmount === 0n;
  const hasRequest = !!backing && backing.withdrawAmount > 0n;
  const canComplete = hasRequest && backing!.withdrawReadyAt <= now;

  return (
    <div className="panel">
      <div>
        <div className="field-group">
          <div className="field-label">
            <span>Back the pool</span>
            <span>no covered wallets, no coverage</span>
          </div>
          <div className="field-input">
            <input placeholder="0" value={amount} onChange={(e) => setAmount(e.target.value)} inputMode="decimal" />
            <span className="unit">USDC</span>
          </div>
        </div>
        <button
          className="primary-action"
          disabled={!client.address || !(Number(amount) > 0) || backAction.isRunning}
          onClick={() => backAction.dispatch()}
        >
          {backAction.isRunning ? "Backing..." : crossChain ? "Back via Circle" : "Back the pool"}
        </button>
        {progress ? <div className="ramp-disclosure" style={{ marginTop: 8 }}>{progress}</div> : null}
        <TxStatus action={backAction} />
        <div className="ramp-disclosure" style={{ marginTop: 8 }}>
          <b>1. Back.</b> New backing waits a short time (maturing) before it can pay claims. When
          it's ready, press "Count my matured backing".
          <br />
          <b>2. Take it out.</b> Press "Request withdrawal", wait out the notice period, then press
          "Complete withdrawal".
          <br />
          Why the waits: nobody can add money just before a claim they know is coming, or pull it
          out right before a claim that's already on the way.
          {crossChain ? " From Ethereum or Solana, your USDC crosses on Circle's bridge and comes home the same way." : ""}
        </div>

        <div className="field-group" style={{ marginTop: 20 }}>
          <div className="field-label">
            <span>Your backing</span>
            <span>&nbsp;</span>
          </div>
          {!backing ? (
            <div className="side-stat">
              <div className="k">Status</div>
              <div className="v">No backing yet</div>
            </div>
          ) : (
            <>
              <div className="side-stat">
                <div className="k">Counting toward capacity</div>
                <div className="v">{fmtUsdc(backing.amount)} USDC</div>
              </div>
              {backing.pendingAmount > 0n ? (
                <div className="side-stat">
                  <div className="k">Maturing</div>
                  <div className="v">
                    {fmtUsdc(backing.pendingAmount)} USDC · ready {when(backing.pendingMaturesAt)}
                  </div>
                </div>
              ) : null}
              {backing.yieldOwed > 0n ? (
                <div className="side-stat">
                  <div className="k">Yield earned</div>
                  <div className="v">{fmtUsdc(backing.yieldOwed)} USDC</div>
                </div>
              ) : null}
              {hasRequest ? (
                <div className="side-stat">
                  <div className="k">Withdrawal requested</div>
                  <div className="v">
                    {fmtUsdc(backing.withdrawAmount)} USDC · ready {when(backing.withdrawReadyAt)}
                  </div>
                </div>
              ) : null}
            </>
          )}
        </div>

        {canMature ? (
          <>
            <button className="secondary-action" style={{ width: "100%" }} disabled={matureAction.isRunning}
              onClick={() => matureAction.dispatch()}>
              {matureAction.isRunning ? "Updating..." : "Count my matured backing"}
            </button>
          </>
        ) : null}
        {/* Results sit outside the conditional blocks: each step hides its own button once done,
            and the result must stay visible (2026-09-25, links vanished right away). */}
        <TxStatus action={matureAction} />
        {canRequest ? (
          <>
            <button className="secondary-action" style={{ width: "100%" }} disabled={requestAction.isRunning}
              onClick={() => requestAction.dispatch()}>
              {requestAction.isRunning ? "Requesting..." : "Request withdrawal"}
            </button>
          </>
        ) : null}
        <TxStatus action={requestAction} />
        {hasRequest ? (
          <>
            <button className="secondary-action" style={{ width: "100%" }}
              disabled={!canComplete || completeAction.isRunning} onClick={() => completeAction.dispatch()}>
              {completeAction.isRunning ? "Withdrawing..." : "Complete withdrawal"}
            </button>
            <button className="secondary-action" style={{ width: "100%", marginTop: 8 }}
              disabled={cancelAction.isRunning} onClick={() => cancelAction.dispatch()}>
              {cancelAction.isRunning ? "Cancelling..." : "Cancel request"}
            </button>
          </>
        ) : null}
        <TxStatus action={completeAction} />
        <TxStatus action={cancelAction} />
        {backing && backing.yieldOwed >= (crossChain ? MIN_BRIDGEABLE_RAW : 1n) ? (
          <button className="secondary-action" style={{ width: "100%", marginTop: 8 }}
            disabled={takeYieldAction.isRunning} onClick={() => takeYieldAction.dispatch()}>
            {takeYieldAction.isRunning ? "Sending..." : `Take ${fmtUsdc(backing.yieldOwed)} USDC yield`}
          </button>
        ) : null}
        <TxStatus action={takeYieldAction} />
      </div>

      <div>
        <div className="side-stat">
          <div className="k">Total staked</div>
          <div className="v">{fmtUsdc(pool.totalStaked)} USDC</div>
        </div>
        <div className="side-stat">
          <div className="k">What backing does</div>
          <div className="v good" style={{ fontFamily: "var(--font-body)", fontSize: 14 }}>
            Backing adds room for claims to be paid. Losses on the yield vault never reach backers.
          </div>
        </div>
      </div>
    </div>
  );
}
