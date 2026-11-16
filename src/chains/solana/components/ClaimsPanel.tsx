import { useEffect, useState } from "react";
import { useAction, useClient } from "../lib/client";
import { approveClaim, collectClaim, releaseQueuedClaim, unlockPendingClaim } from "../lib/actions";
import { num, setting } from "../lib/pool";
import { unstakeFeeBps } from "../lib/program";
import { fmtDuration, fmtSol, readMyClaim, readMyStake, refreshSoon, vested } from "../lib/reads";
import type { ClaimStatus, MyClaim, MyStake } from "../lib/reads";
import { usePool } from "../lib/usePool";
import { FeeNote } from "./StakePanel";
import { TxStatus } from "./TxStatus";

// Collect payout tab. The claim's steps, each a button when it's the next one:
//   Reserved (waiting on the pool's daily room) -> "Try to move it forward" (anyone can)
//   PendingTime (stake younger than the time gate) -> "Start the claim" once the gate passes (anyone can)
//   AwaitingApproval -> "Approve claim" (the staker only; gives up the stake, starts the payout)
//   Active -> "Collect payout" after the cooldown (the staker signs; SOL goes to the staking wallet)
const TIME_GATE = Number(num("TIME_GATE_SECS"));
const TIER_NAME: Record<number, string> = {
  [Number(num("TIER_A"))]: "A",
  [Number(num("TIER_B"))]: "B",
  [Number(num("TIER_C"))]: "C",
};

function statusLabel(status: ClaimStatus): string {
  switch (status) {
    case "Reserved":
      return "In line: accepted, waiting for room in the pool's daily limit";
    case "PendingTime":
      return `Waiting: your stake has to be ${fmtDuration(TIME_GATE)} old before a claim can start`;
    case "AwaitingApproval":
      return "Ready to approve";
    case "Active":
      return "Paying out";
    case "Completed":
      return "Paid in full";
    case "Expired":
      return "Expired: the time to approve ran out";
    case "Cancelled":
      return "Cancelled by the SAFU team";
    default:
      return "No claim";
  }
}

export function ClaimsPanel() {
  const client = useClient();
  const staker = client.address;
  const [refreshKey, setRefreshKey] = useState(0);
  const { pool, now } = usePool(refreshKey);
  // Live settings: what an approval now would start with.
  const COOLDOWN = pool ? Number(setting(pool, "COOLDOWN_SECS")) : 0;
  const VESTING = pool ? Number(setting(pool, "VESTING_SECS")) : 0;
  const [stake, setStake] = useState<MyStake | null>(null);
  const [claim, setClaim] = useState<MyClaim | null>(null);
  const [feeBps, setFeeBps] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!pool || !staker) {
        setStake(null);
        setClaim(null);
        return;
      }
      const s = await readMyStake(pool, staker).catch(() => null);
      const c = await readMyClaim(s).catch(() => null);
      const left = c ? c.data.entitlement - c.data.streamed : 0n;
      const fee = c && c.data.status === "Active" && left > 0n ? await unstakeFeeBps(pool, left).catch(() => null) : null;
      if (!cancelled) {
        setStake(s);
        setClaim(c);
        setFeeBps(fee);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pool, staker]);

  const done = <T,>(p: Promise<T>) => p.finally(() => refreshSoon(() => setRefreshKey((k) => k + 1)));
  const need = () => {
    if (!staker || !claim) throw new Error("You don't have a claim open.");
    return { staker, claim: claim.address };
  };
  const releaseAction = useAction(() => done((async () => { const n = need(); return releaseQueuedClaim(client, pool, n.staker, n.claim); })()));
  const unlockAction = useAction(() => done((async () => { const n = need(); return unlockPendingClaim(client, pool, n.staker, n.claim); })()));
  const approveAction = useAction(() => done((async () => approveClaim(client, pool, need().claim))()));
  const collectAction = useAction(() => done((async () => collectClaim(client, pool, need().claim, stake?.beneficiary ?? staker ?? ""))()));

  const status = claim?.data.status;
  const gateOpensAt = stake ? Number(stake.staked_at) + TIME_GATE : 0;
  const cooldownLeft = claim ? Number(claim.data.cooldown_ends) - now : 0;
  const collectable = claim && status === "Active" ? vested(claim.data, now) - claim.data.streamed : 0n;

  return (
    <div className="panel">
      <div>
        <div className="field-group">
          <div className="field-label">
            <span>Your claim</span>
            <span>&nbsp;</span>
          </div>
          <div className="side-stat">
            <div className="k">Status</div>
            <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 14 }}>
              {claim ? statusLabel(claim.data.status) : staker ? "No open claim" : "Connect the wallet you staked with"}
            </div>
          </div>
        </div>

        {status === "Reserved" ? (
          <button className="primary-action" disabled={releaseAction.isRunning} onClick={() => releaseAction.dispatch()}>
            {releaseAction.isRunning ? "Checking..." : "Try to move it forward"}
          </button>
        ) : null}
        <TxStatus action={releaseAction} />

        {status === "PendingTime" ? (
          <>
            <div className="ramp-disclosure" style={{ marginTop: 8 }}>
              {gateOpensAt > now ? `This can start in ${fmtDuration(gateOpensAt - now)}.` : "The wait is over. Start the claim."}
            </div>
            <button className="primary-action" disabled={gateOpensAt > now || unlockAction.isRunning} onClick={() => unlockAction.dispatch()}>
              {unlockAction.isRunning ? "Starting..." : "Start the claim"}
            </button>
          </>
        ) : null}
        <TxStatus action={unlockAction} />

        {status === "AwaitingApproval" ? (
          <>
            <div className="ramp-disclosure" style={{ marginTop: 8 }}>
              Approving gives up your staked SOL and starts the payout: a {fmtDuration(COOLDOWN)} wait, then
              it pays out over {fmtDuration(VESTING)}. Only you can do this, it can't be undone, and it has to
              happen by {new Date(Number(claim!.data.approve_deadline) * 1000).toLocaleString()}.
            </div>
            <button className="primary-action" disabled={approveAction.isRunning} onClick={() => approveAction.dispatch()}>
              {approveAction.isRunning ? "Approving..." : "Approve claim"}
            </button>
          </>
        ) : null}
        <TxStatus action={approveAction} />

        <div className="field-group" style={{ marginTop: 20 }}>
          <div className="field-label">
            <span>Payout</span>
            <span>&nbsp;</span>
          </div>
          {status === "Active" && cooldownLeft > 0 ? (
            <div className="ramp-disclosure" style={{ marginBottom: 8 }}>Payout starts in {fmtDuration(cooldownLeft)}.</div>
          ) : null}
          <button
            className="primary-action"
            disabled={status !== "Active" || cooldownLeft > 0 || collectable <= 0n || collectAction.isRunning}
            onClick={() => collectAction.dispatch()}
          >
            {collectAction.isRunning ? "Sending..." : collectable > 0n ? `Collect ${fmtSol(collectable)} SOL` : "Collect payout"}
          </button>
          {status === "Active" ? <FeeNote bps={feeBps} /> : null}
        </div>
        <TxStatus action={collectAction} />
      </div>
      <div>
        {claim ? (
          <>
            <div className="side-stat">
              <div className="k">Payout total</div>
              <div className="v">{fmtSol(claim.data.entitlement)} SOL</div>
            </div>
            <div className="side-stat">
              <div className="k">Paid so far</div>
              <div className="v">{fmtSol(claim.data.streamed)} SOL</div>
            </div>
            <div className="side-stat">
              <div className="k">Tier</div>
              <div className="v">{TIER_NAME[claim.data.tier] ?? "--"}</div>
            </div>
          </>
        ) : (
          <div className="side-stat">
            <div className="k">How this works</div>
            <div className="v good">
              After you approve, the payout builds up over time. Each time you press Collect, whatever has built
              up so far is sent to your staking wallet, in SOL.
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
