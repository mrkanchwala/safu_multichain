import { useEffect, useState } from "react";
import { useAction, useClient } from "../lib/client";
import { relayAction, relayHomeAction } from "../lib/crosschain";
import { useStaker } from "../lib/useStaker";
import { useTick } from "../lib/useTick";
import type { AppClient } from "../lib/client";
import { approveClaim, pullClaimPayout } from "../lib/actions";
import { ClaimStatus, fmtUsdc, readActiveClaim } from "../lib/reads";
import { TxStatus } from "./TxStatus";

// FIXED 2026-09-19 (A9): labels now match the REAL 8-state ClaimStatus
// (types.rs:246-266) and its actual mechanics -- a 90-day time gate
// (TIME_GATE_LEDGERS), not the old mock's invented "60-day gate", and a
// separate 100-day approval window (APPROVE_WINDOW_LEDGERS) the STAKER
// themselves must act within, which the old mock's status set had no
// equivalent for at all.
function statusLabel(status: ClaimStatus): string {
  switch (status) {
    case ClaimStatus.Unused:
      return "No claim filed against this stake";
    case ClaimStatus.PendingTime:
      return "Queued: held until the 90-day gate clears, nothing forfeited yet";
    case ClaimStatus.AwaitingApproval:
      return "Ready to approve: do it within 100 days to forfeit your principal and start the payout";
    case ClaimStatus.Reserved:
      return "Reserved: admitted but waiting on daily capacity";
    case ClaimStatus.Active:
      return "Active: streaming, pull below";
    case ClaimStatus.Completed:
      return "Completed: fully paid";
    case ClaimStatus.Expired:
      return "Expired: the 100-day approval window lapsed with no action";
    case ClaimStatus.Cancelled:
      return "Cancelled by admin override";
    default:
      return "Unknown";
  }
}

// Collect payout tab (2026-09-24): the old "Manage claim" view, promoted to a main tab. Filing moved
// to its own tab (ClaimFilePanel).
export function ClaimsPanel() {
  const client = useClient<AppClient>();
  // Stellar staker: acts directly. EVM/Solana staker: its safu-account, owner-signed via the relayer;
  // the payout goes home over CCTP (`claim_home`) to the wallet's own chain (2026-09-24).
  const { address: staker, crossChain } = useStaker(client, "stake");
  const [claim, setClaim] = useState<Awaited<ReturnType<typeof readActiveClaim>>>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const tick = useTick();

  useEffect(() => {
    if (!staker) {
      setClaim(null);
      return;
    }
    let cancelled = false;
    (async () => {
      const c = await readActiveClaim(client, staker);
      if (!cancelled) setClaim(c);
    })();
    return () => {
      cancelled = true;
    };
  }, [client, staker, refreshKey, tick]);

  const pullAction = useAction(async () => {
    if (!staker) throw new Error("Connect a wallet first.");
    if (!claim) throw new Error("You don't have a claim to collect.");
    const sig = crossChain
      ? await relayHomeAction(client, staker, "claim_home", { claim_id: claim.claimIdHex })
      : await pullClaimPayout(client, null, claim.claimIdHex);
    setRefreshKey((k) => k + 1);
    return sig;
  });

  const approveAction = useAction(async () => {
    if (!staker) throw new Error("Connect a wallet first.");
    if (!claim) throw new Error("You don't have a claim to approve.");
    const sig = crossChain
      ? await relayAction(client, staker, "approve_claim", { claim_id: claim.claimIdHex })
      : await approveClaim(client, null, claim.claimIdHex);
    setRefreshKey((k) => k + 1);
    return sig;
  });

  const canApprove = claim && claim.data.status === ClaimStatus.AwaitingApproval;
  const canPull = claim && claim.data.status === ClaimStatus.Active;

  return (
    <div className="panel">
      <div>
        <>
            <div className="field-group">
              <div className="field-label">
                <span>Your claim</span>
                <span>&nbsp;</span>
              </div>
              {!claim ? (
                <div className="side-stat">
                  <div className="k">Status</div>
                  <div className="v">No active claim</div>
                </div>
              ) : (
                <div className="side-stat">
                  <div className="k">Status</div>
                  <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 14 }}>
                    {statusLabel(claim.data.status)}
                  </div>
                </div>
              )}
            </div>
            {canApprove ? (
              <>
                <div className="ramp-disclosure" style={{ marginTop: 8 }}>
                  Approving forfeits your staked principal and starts the 7-day cooldown before
                  streaming begins. Only you can take this step, and it can't be undone.
                </div>
                <button
                  className="primary-action"
                  disabled={!staker || approveAction.isRunning}
                  onClick={() => approveAction.dispatch()}
                >
                  {approveAction.isRunning ? "Approving..." : "Approve claim"}
                </button>
              </>
            ) : null}
            {/* Outside the block above: approving hides it, and the result must stay visible. */}
            <TxStatus action={approveAction} />

            <div className="field-group" style={{ marginTop: 20 }}>
              <div className="field-label">
                <span>Payout</span>
                <span>&nbsp;</span>
              </div>
              <button
                className="primary-action"
                disabled={!staker || !canPull || pullAction.isRunning}
                onClick={() => pullAction.dispatch()}
              >
                {pullAction.isRunning ? "Sending..." : crossChain ? "Collect payout to my wallet" : "Collect payout"}
              </button>
            </div>
            <TxStatus action={pullAction} />
        </>
      </div>
      <div>
        {claim ? (
          <>
            <div className="side-stat">
              <div className="k">Entitlement (on-chain)</div>
              <div className="v">{fmtUsdc(claim.data.entitlement)} USDC</div>
            </div>
            <div className="side-stat">
              <div className="k">Streamed so far</div>
              <div className="v">{fmtUsdc(claim.data.streamed)} USDC</div>
            </div>
          </>
        ) : (
          <div className="side-stat">
            <div className="k">How this works</div>
            <div className="v good">
              Anyone can trigger a pull. The payout streams continuously, and each pull sends you
              whatever has vested so far, in USDC.
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
