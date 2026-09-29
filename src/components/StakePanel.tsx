import { useEffect, useState } from "react";
import { POOL } from "../lib/pool";
import { useAction, useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import { stake, withdrawStake } from "../lib/actions";
import { addCoveredWallet, confirmCoveredWallet } from "../lib/claimApi";
import type { CoveredWalletResponse } from "../lib/claimApi";
import {
  evmBurnToAdapter,
  evmUsdcBalance,
  finishOnStellar,
  relayHomeAction,
  solanaBurnToAdapter,
  solanaUsdcBalance,
} from "../lib/crosschain";
import { fmtUsdc, readCoveredWallets, readMyStake, readPoolStats, readYieldIndex, rememberCoveredWallet } from "../lib/reads";
import type { CoveredWallet, MyStake } from "../lib/reads";
import { CHAINS, MAX_STAKE_USDC, MIN_STAKE_USDC, POOL_CAP_USDC, YIELD_INDEX_PRECISION } from "../lib/network";
import type { ChainId } from "../lib/network";
import { useStaker } from "../lib/useStaker";
import { useTick } from "../lib/useTick";
import { toFriendlyError } from "../lib/friendly-error";
import { TxStatus } from "./TxStatus";

// Stake tab. Rebuilt 2026-09-24 (P5 Stage 2) for all three wallet families:
//   Stellar wallet  -> `stake` directly, signed by the wallet kit (unchanged).
//   EVM / Solana    -> USDC burned on the home chain to the stake adapter over Circle's CCTP; the
//                      relayer finishes it on Stellar, where the wallet's own safu-account stakes.
// Covered wallets use the ownership proof (2026-09-23): add -> send the shown amount from that
// wallet to itself -> "Sent" -> registered. The covered-wallet cap (3) matches registry.py.
const MAX_COVERED_WALLETS = 3;

function yieldStripPct(withdrawable: bigint, principal: bigint): string {
  if (principal <= 0n) return "0.00";
  const bps = ((withdrawable - principal) * 10_000n) / principal;
  return (Number(bps) / 100).toFixed(2);
}

export function StakePanel() {
  const client = useClient<AppClient>();
  const { address: staker, crossChain } = useStaker(client, "stake");

  const [poolStats, setPoolStats] = useState({ totalStaked: 0n, totalStakers: 0 });
  const [myStake, setMyStake] = useState<MyStake | null>(null);
  const [yieldIndex, setYieldIndex] = useState<bigint>(BigInt(YIELD_INDEX_PRECISION));
  const [coveredWallets, setCoveredWallets] = useState<CoveredWallet[]>([]);
  const [homeUsdc, setHomeUsdc] = useState<bigint | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const tick = useTick();

  const [stakeAmount, setStakeAmount] = useState("");
  const [progress, setProgress] = useState<string | null>(null);
  const [newChain, setNewChain] = useState<ChainId>(CHAINS[0].id);
  const [newWallet, setNewWallet] = useState("");
  const [pending, setPending] = useState<CoveredWalletResponse | null>(null);
  const [walletMsg, setWalletMsg] = useState<string | null>(null);
  const [walletBusy, setWalletBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [stats, idx] = await Promise.all([readPoolStats(client), readYieldIndex(client)]);
      if (!cancelled) {
        setPoolStats(stats);
        setYieldIndex(idx);
      }
      if (staker) {
        const [s, wallets] = await Promise.all([readMyStake(client, staker), readCoveredWallets(staker)]);
        if (!cancelled) {
          setMyStake(s);
          setCoveredWallets(wallets);
        }
      } else if (!cancelled) {
        setMyStake(null);
        setCoveredWallets([]);
      }
      if (client.address && client.kind === "evm") {
        const b = await evmUsdcBalance(client.address).catch(() => null);
        if (!cancelled) setHomeUsdc(b);
      } else if (client.address && client.kind === "solana") {
        const b = await solanaUsdcBalance(client.address).catch(() => null);
        if (!cancelled) setHomeUsdc(b);
      } else if (!cancelled) setHomeUsdc(null);
    })();
    return () => {
      cancelled = true;
    };
  }, [client, staker, refreshKey, tick]);

  const amountValid = Number(stakeAmount) >= MIN_STAKE_USDC && Number(stakeAmount) <= MAX_STAKE_USDC;

  const stakeAction = useAction(async (signal) => {
    if (!client.address) throw new Error("Connect a wallet first.");
    try {
      if (client.kind === "stellar") {
        // Beneficiary == staker (stake.rs, 2026-09-19): payouts land back on the wallet that staked.
        return await stake(client, null, stakeAmount, client.address);
      }
      const kind = client.kind as "evm" | "solana";
      const usdc6 = BigInt(Math.round(Number(stakeAmount) * 1e6));
      setProgress(`Burning ${stakeAmount} USDC on ${kind === "evm" ? POOL.evm.label : POOL.solana.label}...`);
      const burn = kind === "evm"
        ? await evmBurnToAdapter(client, usdc6, "stake")
        : await solanaBurnToAdapter(client, usdc6, "stake");
      const account = await finishOnStellar("stake", kind, burn, setProgress, signal);
      return `Staked from your safu-account ${account.slice(0, 6)}...`;
    } finally {
      setProgress(null);
      setStakeAmount("");
      setRefreshKey((k) => k + 1);
    }
  });

  const withdrawAction = useAction(async () => {
    if (!client.address || !staker) throw new Error("Connect a wallet first.");
    try {
      if (client.kind === "stellar") return await withdrawStake(client, null, client.address);
      // withdraw_home: unstake and send the USDC back over CCTP to the wallet's own chain.
      return await relayHomeAction(client, staker, "withdraw_home", {}, setProgress);
    } finally {
      setProgress(null);
      setRefreshKey((k) => k + 1);
    }
  });

  function registered(r: CoveredWalletResponse) {
    if (!staker) return;
    const row = rememberCoveredWallet(staker, r.chain, r.wallet, r.registered_at ?? Math.floor(Date.now() / 1000));
    setCoveredWallets((cur) => (cur.some((w) => w.chain === row.chain && w.wallet === row.wallet) ? cur : [...cur, row]));
    setPending(null);
    setNewWallet("");
    setWalletMsg(r.method === "link" ? "Registered (linked by a past transfer)." : "Registered.");
  }

  async function addWallet() {
    setWalletMsg(null);
    if (!staker) return setWalletMsg(crossChain ? "Stake first: your safu-account covers the wallets." : "Connect a wallet first.");
    if (coveredWallets.length >= MAX_COVERED_WALLETS) return setWalletMsg(`You can cover at most ${MAX_COVERED_WALLETS} wallets.`);
    if (!newWallet.trim()) return setWalletMsg("Enter a wallet address.");
    if (newChain === "stellar" && newWallet.trim() === staker) {
      return setWalletMsg("A covered wallet can't be the same as your staking wallet.");
    }
    setWalletBusy(true);
    try {
      const r = await addCoveredWallet(client, staker, newChain, newWallet.trim());
      if (r.status === "registered") registered(r);
      else setPending(r);
    } catch (e) {
      setWalletMsg(toFriendlyError(e).message);
    } finally {
      setWalletBusy(false);
    }
  }

  async function confirmSent() {
    if (!pending || !staker) return;
    setWalletBusy(true);
    setWalletMsg(null);
    try {
      const r = await confirmCoveredWallet(staker, pending.chain, pending.wallet);
      if (r.status === "registered") registered(r);
      else setWalletMsg("Not on-chain yet. Give it a few seconds after sending, then press Sent again.");
    } catch (e) {
      setWalletMsg(toFriendlyError(e).message);
    } finally {
      setWalletBusy(false);
    }
  }

  const chainLabel = (id: string) => CHAINS.find((c) => c.id === id)?.label ?? id;

  return (
    <div className="panel">
      <div>
        <div className="field-group">
          <div className="field-label">
            <span>Stake</span>
            <span>${MIN_STAKE_USDC}–${MAX_STAKE_USDC.toLocaleString()}</span>
          </div>
          <div className="field-input">
            <input placeholder="0" value={stakeAmount} onChange={(e) => setStakeAmount(e.target.value)} inputMode="decimal" />
            <span className="unit">USDC</span>
          </div>
        </div>
        <button
          className="primary-action"
          disabled={!client.address || !amountValid || stakeAction.isRunning}
          onClick={() => stakeAction.dispatch()}
        >
          {stakeAction.isRunning ? "Staking..." : crossChain ? "Stake via Circle" : "Stake"}
        </button>
        {progress ? <div className="ramp-disclosure" style={{ marginTop: 8 }}>{progress}</div> : null}
        <TxStatus action={stakeAction} />
        <div className="ramp-disclosure" style={{ marginTop: 8 }}>
          {crossChain
            ? `Your USDC crosses to Stellar on Circle's bridge and your own safu-account stakes it. Only your wallet's signature can move it.${homeUsdc !== null ? ` USDC in your wallet: ${(Number(homeUsdc) / 1e6).toFixed(2)}.` : ""}`
            : "Payouts go back to this same wallet. Stake from a wallet you're comfortable connecting here, and cover your other wallets below without ever connecting them."}
        </div>

        <div className="field-group" style={{ marginTop: 20 }}>
          <div className="field-label">
            <span>Your stake</span>
            <span>&nbsp;</span>
          </div>
          {!myStake ? (
            <div className="side-stat">
              <div className="k">Status</div>
              <div className="v">No active stake</div>
            </div>
          ) : (
            <>
              <div className="side-stat">
                <div className="k">Principal (entitlement basis)</div>
                <div className="v">{fmtUsdc(myStake.amount)} USDC</div>
              </div>
              <div className="side-stat">
                <div className="k">Withdrawable now (principal + yield)</div>
                <div className="v">
                  {fmtUsdc(myStake.withdrawable)} USDC{" "}
                  <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
                    (+{yieldStripPct(myStake.withdrawable, myStake.amount)}%)
                  </span>
                </div>
              </div>
            </>
          )}
        </div>
        <button
          className="secondary-action"
          style={{ width: "100%" }}
          disabled={!client.address || !myStake || withdrawAction.isRunning}
          onClick={() => withdrawAction.dispatch()}
        >
          {withdrawAction.isRunning ? "Withdrawing..." : crossChain ? "Withdraw to my wallet" : "Withdraw principal + yield"}
        </button>
        <TxStatus action={withdrawAction} />

        <div className="field-group" style={{ marginTop: 20 }}>
          <div className="field-label">
            <span>Covered wallets</span>
            <span>{coveredWallets.length}/{MAX_COVERED_WALLETS}</span>
          </div>
          <div className="ramp-disclosure">
            Register a wallet before it's drained, because a claim can only name a wallet already on this list.
            Any mix of chains, up to {MAX_COVERED_WALLETS}. To prove it's yours, that wallet sends a tiny
            amount to itself.
          </div>
          {coveredWallets.map((w, i) => (
            <div className="side-stat" key={`${w.chain}-${w.wallet}-${i}`}>
              <div className="k">{chainLabel(w.chain)}</div>
              <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 13 }}>{w.wallet}</div>
            </div>
          ))}

          {pending ? (
            <div className="side-stat" style={{ marginTop: 10 }}>
              <div className="k">Prove you own this {chainLabel(pending.chain)} wallet</div>
              <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 13 }}>
                From <b>{pending.wallet}</b>, send exactly <b>{pending.amount} {pending.asset}</b> to the same
                address (to itself). Then press Sent.
                {pending.expires_at ? ` Expires ${new Date(pending.expires_at * 1000).toLocaleTimeString()}.` : ""}
              </div>
              <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={walletBusy} onClick={confirmSent}>
                {walletBusy ? "Checking..." : "Sent"}
              </button>
              <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={walletBusy}
                onClick={() => setPending(null)}>
                Cancel
              </button>
            </div>
          ) : coveredWallets.length < MAX_COVERED_WALLETS ? (
            <>
              <div className="ramp-toggle" style={{ marginTop: 10, marginBottom: 8 }}>
                {CHAINS.map((c) => (
                  <button key={c.id} className={`ramp-toggle-btn${newChain === c.id ? " active" : ""}`} onClick={() => setNewChain(c.id)}>
                    {c.label}
                  </button>
                ))}
              </div>
              <div className="field-input">
                <input placeholder="Wallet address on that chain" value={newWallet} onChange={(e) => setNewWallet(e.target.value)} />
              </div>
              <button className="secondary-action" style={{ width: "100%" }} disabled={walletBusy} onClick={addWallet}>
                {walletBusy ? "Checking..." : "Add covered wallet"}
              </button>
            </>
          ) : null}
          {walletMsg ? (
            <div className="tx-status" style={{ marginTop: 8 }}>
              <div>{walletMsg}</div>
            </div>
          ) : null}
        </div>
      </div>

      <div>
        <div className="side-stat">
          <div className="k">Pool cap</div>
          <div className="v">${POOL_CAP_USDC.toLocaleString()}</div>
        </div>
        <div className="side-stat">
          <div className="k">Total staked</div>
          <div className="v">{fmtUsdc(poolStats.totalStaked)} USDC</div>
        </div>
        <div className="side-stat">
          <div className="k">Stakers</div>
          <div className="v">{poolStats.totalStakers}</div>
        </div>
        {staker && crossChain ? (
          <div className="side-stat">
            <div className="k">Your safu-account (Stellar)</div>
            <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 12 }}>{staker}</div>
          </div>
        ) : null}
        <div className="side-stat">
          <div className="k">Yield earned so far</div>
          <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 14 }}>
            +{((Number(yieldIndex) / Number(YIELD_INDEX_PRECISION) - 1) * 100).toFixed(2)}%
          </div>
        </div>
        <div className="side-stat">
          <div className="k">Coverage ceilings</div>
          <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 13 }}>
            Up to 15x (Tier A) &middot; up to 10x (Tier B) &middot; up to 5x (Tier C) of your principal, never more.
          </div>
        </div>
      </div>
    </div>
  );
}
