import { useEffect, useState } from "react";
import { POOL } from "../lib/pool";
import { useAction, useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import { claimStakerYield, stake, withdrawStake } from "../lib/actions";
import { addCoveredWallet, confirmCoveredWallet, countCoveredWallets, listMyCoveredWallets } from "../lib/claimApi";
import { walletChainMismatch } from "../lib/wallet-chain";
import type { CoveredWalletResponse, CoveredWalletTail } from "../lib/claimApi";
import { FAQ_ANSWER, faqAnswerHref } from "../lib/links";
import {
  evmBurnToAdapter,
  evmUsdcBalance,
  finishOnStellar,
  relayHomeAction,
  burnForStake,
  solanaBurnToAdapter,
  solanaUsdcBalance,
} from "../lib/crosschain";
import { fmtExact, fmtUsdc, fmtYield, readCoveredWallets, readMyStake, readPoolStats, readYieldIndex, rememberCoveredWallet } from "../lib/reads";
import type { CoveredWallet, MyStake } from "../lib/reads";
import { CHAINS, MAX_STAKE_USDC, MIN_BRIDGEABLE_RAW, MIN_STAKE_USDC, POOL_CAP_USDC, YIELD_INDEX_PRECISION } from "../lib/network";
import type { ChainId } from "../lib/network";
import { useStaker } from "../lib/useStaker";
import { useTick } from "../lib/useTick";
import { toFriendlyError } from "../lib/friendly-error";
import { TxStatus } from "./TxStatus";

// Stake tab. Rebuilt 2026-09-24 (multichain build) for all three wallet families:
//   Stellar wallet  -> `stake` directly, signed by the wallet kit (unchanged).
//   EVM / Solana    -> USDC burned on the home chain to the stake adapter over Circle's CCTP; the
//                      relayer finishes it on Stellar, where the wallet's own safu-account stakes.
// Covered wallets use the ownership proof (2026-09-23): add -> send the shown amount from that
// wallet to itself -> "Sent" -> registered. The covered-wallet cap (3) matches registry.py.
const MAX_COVERED_WALLETS = 3;

function yieldStripPct(withdrawable: bigint, principal: bigint): string {
  if (principal <= 0n) return "0.0000";
  // Four decimals: on a small stake a few days of yield is well under 0.01%.
  const ppm = ((withdrawable - principal) * 1_000_000n) / principal;
  return (Number(ppm) / 10_000).toFixed(4);
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
  // B19: the count comes from the server (any browser); null = still checking or the read failed,
  // which never shows green. Tails = last 4 characters, only after the staker signs.
  const [coveredCount, setCoveredCount] = useState<number | null>(null);
  const [tails, setTails] = useState<CoveredWalletTail[] | null>(null);
  const [tailsMsg, setTailsMsg] = useState<string | null>(null);
  const [tailsBusy, setTailsBusy] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setCoveredCount(null);
    setTails(staker ? readCachedTails(staker) : null);
    if (staker) {
      countCoveredWallets(staker)
        .then((count) => !cancelled && setCoveredCount(count))
        .catch(() => !cancelled && setCoveredCount(null));
    }
    return () => {
      cancelled = true;
    };
  }, [staker, refreshKey]);

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
      // Circle's fee comes out of the burn, so it is sent on top: the typed amount is what gets staked.
      const burn6 = burnForStake(usdc6, BigInt(MIN_STAKE_USDC * 1e6), BigInt(MAX_STAKE_USDC * 1e6));
      const sending = String(Number(burn6) / 1e6);
      if (homeUsdc !== null && burn6 > homeUsdc) {
        throw new Error(`You don't have enough USDC in this wallet. Staking ${stakeAmount} USDC sends ${sending}, including Circle's fee.`);
      }
      setProgress(`Sending ${sending} USDC on ${kind === "evm" ? POOL.evm.label : POOL.solana.label} (${stakeAmount} to stake + Circle's fee)...`);
      const burn = kind === "evm"
        ? await evmBurnToAdapter(client, burn6, "stake", setProgress)
        : await solanaBurnToAdapter(client, burn6, "stake");
      const account = await finishOnStellar("stake", kind, burn, setProgress, signal);
      // A deposit that doesn't fit the pool's limits waits in the safu-account unstaked (cctp-adapter),
      // so "Staked" is said only once the pool has the stake.
      if (!(await stakeLanded(client, account))) {
        throw new Error("Your USDC reached your safu-account but wasn't staked. It's safe there. Message the SAFU team and we'll help you finish it.");
      }
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

  // r3: take the yield, keep the stake. Stellar: paid to the staking wallet itself (the
  // beneficiary, stake.rs 2026-09-19). EVM / Solana: claim_yield_home sends it over CCTP.
  const takeYieldAction = useAction(async () => {
    if (!client.address || !staker) throw new Error("Connect a wallet first.");
    try {
      if (client.kind === "stellar") return await claimStakerYield(client, client.address);
      return await relayHomeAction(client, staker, "claim_yield_home", {}, setProgress);
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
    setAddOpen(false);
    setRefreshKey((k) => k + 1);
  }

  async function showAddresses() {
    if (!staker) return;
    setTailsBusy(true);
    setTailsMsg("Sign once to see the last 4 characters. Only you can see them.");
    try {
      const t = await listMyCoveredWallets(client, staker);
      setTails(t);
      cacheTails(staker, t);
      setTailsMsg(null);
    } catch {
      setTailsMsg("Couldn't load them. Try again.");
    } finally {
      setTailsBusy(false);
    }
  }

  async function addWallet() {
    setWalletMsg(null);
    if (!staker) return setWalletMsg(crossChain ? "Stake first: your safu-account covers the wallets." : "Connect a wallet first.");
    if (coveredWallets.length >= MAX_COVERED_WALLETS) return setWalletMsg(`You can cover at most ${MAX_COVERED_WALLETS} wallets.`);
    if (!newWallet.trim()) return setWalletMsg("Enter a wallet address.");
    const mismatch = walletChainMismatch(newChain, newWallet);
    if (mismatch) return setWalletMsg(mismatch);
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

  // Section 2 state. Green needs an active stake AND at least one wallet the server counts.
  const staked = myStake !== null;
  const n = coveredCount;
  const protectedNow = staked && n !== null && n > 0;
  const coverEnded = !staked && n !== null && n > 0;
  const full = n !== null && n >= MAX_COVERED_WALLETS;

  // Rows: wallets this browser added (full address known), then the server's last-4 list for the
  // rest, then plain "Wallet N" rows for any the count has that neither of those can name.
  const rows: { key: string; label: string; addr: string }[] = coveredWallets.map((w, i) => ({
    key: `l-${w.chain}-${w.wallet}-${i}`, label: chainLabel(w.chain), addr: shortAddr(w.wallet),
  }));
  for (const t of tails ?? []) {
    if (!coveredWallets.some((w) => w.chain === t.chain && w.wallet.endsWith(t.tail))) {
      rows.push({ key: `t-${t.chain}-${t.tail}`, label: chainLabel(t.chain), addr: `…${t.tail}` });
    }
  }
  // Never more rows than the server counts: a stale cache must not show "Covered" under 0/3.
  if (n !== null) rows.length = Math.min(rows.length, n);
  const unnamed = n !== null ? Math.max(0, n - rows.length) : 0;
  for (let i = 0; i < unnamed; i++) rows.push({ key: `u-${i}`, label: `Wallet ${rows.length + 1}`, addr: "" });

  let s2Right: { text: string; cls: string };
  if (!staked) s2Right = { text: n !== null ? `${n}/${MAX_COVERED_WALLETS}` : `0/${MAX_COVERED_WALLETS}`, cls: "" };
  else if (n === null) s2Right = { text: "Checking…", cls: "" };
  else if (n === 0) s2Right = { text: `0/${MAX_COVERED_WALLETS} · Not protected yet`, cls: "warn" };
  else s2Right = { text: `${n}/${MAX_COVERED_WALLETS} · Protected`, cls: "ok" };

  const addForm = (
    <>
      <div className="ramp-toggle" style={{ marginTop: 10, marginBottom: 8 }}>
        {CHAINS.map((c) => (
          <button key={c.id} className={`ramp-toggle-btn${newChain === c.id ? " active" : ""}`} onClick={() => setNewChain(c.id)}>
            {c.label}
          </button>
        ))}
      </div>
      <div className="field-input">
        <input className="cover-addr" placeholder="Wallet address on that chain" value={newWallet} onChange={(e) => setNewWallet(e.target.value)} />
      </div>
      <button className="secondary-action" style={{ width: "100%" }} disabled={walletBusy} onClick={addWallet}>
        {walletBusy ? "Checking..." : "Add covered wallet"}
      </button>
      <div className="step-hint">
        Up to {MAX_COVERED_WALLETS} wallets on any chain. Each one sends a tiny amount to itself to prove it's yours.
        Add them before anything happens.{" "}
        <a href={faqAnswerHref(FAQ_ANSWER.addCovered)}>What's a covered wallet?</a>
      </div>
    </>
  );

  return (
    <div className="panel">
      <div>
        {/* 1 Stake: open until staked, then one line; Manage stake reopens it. */}
        {!staked ? (
          <div className="step active">
            <div className="step-h">
              <span className="step-n">1</span>Stake
              <span className="step-r">${MIN_STAKE_USDC}–${MAX_STAKE_USDC.toLocaleString()}</span>
            </div>
            <div className="field-input" style={{ marginTop: 12 }}>
              <input placeholder="0" value={stakeAmount} onChange={(e) => setStakeAmount(e.target.value)} inputMode="decimal" />
              <span className="unit">USDC</span>
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
            <div className="step-hint">
              Use a fresh wallet you keep safe. It gets your payouts but isn't covered itself.{" "}
              <a href={faqAnswerHref(FAQ_ANSWER.whichWallet)}>Which wallet?</a>
            </div>
            {crossChain ? (
              <div className="step-hint">
                Your USDC crosses to Stellar on Circle's bridge and your own safu-account stakes it.
                {homeUsdc !== null ? ` USDC in your wallet: ${(Number(homeUsdc) / 1e6).toFixed(2)}.` : ""}
              </div>
            ) : null}
          </div>
        ) : !manageOpen ? (
          <div className="step done">
            <div className="step-h">
              <span className="step-n">✓</span>Staked {fmtUsdc(myStake.amount)} USDC
              <button className="step-link" onClick={() => setManageOpen(true)}>Manage stake</button>
            </div>
          </div>
        ) : (
          <div className="step active">
            <div className="step-h">
              <span className="step-n">1</span>Your stake
              <button className="step-link" onClick={() => setManageOpen(false)}>Close ✕</button>
            </div>
            <div className="step-rows">
              <div className="step-row"><span className="k">Staked</span><span className="v">{fmtUsdc(myStake.amount)} USDC</span></div>
              <div className="step-row">
                <span className="k">Withdrawable now</span>
                <span className="v">{fmtExact(myStake.withdrawable)} USDC (+{yieldStripPct(myStake.withdrawable, myStake.amount)}%)</span>
              </div>
              {staker && crossChain ? (
                <div className="step-row">
                  <span className="k">Your safu-account (Stellar)</span>
                  <span className="v addr-copy"><span title={staker}>{shortAddr(staker)}</span><CopyButton text={staker} /></span>
                </div>
              ) : null}
            </div>
            {myStake.yieldOwed >= (crossChain ? MIN_BRIDGEABLE_RAW : 1n) ? (
              <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={takeYieldAction.isRunning}
                onClick={() => takeYieldAction.dispatch()}>
                {takeYieldAction.isRunning ? "Sending..." : `Take ${fmtYield(myStake.yieldOwed)} USDC yield, keep my stake`}
              </button>
            ) : null}
            <TxStatus action={takeYieldAction} />
            <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={withdrawAction.isRunning}
              onClick={() => withdrawAction.dispatch()}>
              {withdrawAction.isRunning ? "Withdrawing..." : crossChain ? "Withdraw to my wallet" : "Withdraw principal + yield"}
            </button>
            {progress ? <div className="ramp-disclosure" style={{ marginTop: 8 }}>{progress}</div> : null}
            <TxStatus action={withdrawAction} />
            <div className="step-warn">Withdrawing ends cover for all your wallets.</div>
          </div>
        )}

        {/* 2 Cover your wallets: locked until staked; folds to one line while Manage stake is open. */}
        {staked && manageOpen ? (
          <div className={`step${protectedNow ? " done" : ""}`}>
            <div className="step-h">
              <span className="step-n">{protectedNow ? "✓" : "2"}</span>Cover your wallets
              <span className={`step-r ${s2Right.cls}`}>{s2Right.text}</span>
            </div>
          </div>
        ) : !staked ? (
          <div className="step locked">
            <div className="step-h">
              <span className="step-n">2</span>Cover your wallets
              <span className="step-r">{s2Right.text}</span>
            </div>
            <div className="step-hint">
              {coverEnded ? "Cover ended when you withdrew. Stake again to bring it back." : "Stake first, then add the wallets you want covered."}
            </div>
          </div>
        ) : (
          <div className={`step ${protectedNow ? "done" : "active"}`}>
            <div className="step-h">
              <span className="step-n">{protectedNow ? "✓" : "2"}</span>Cover your wallets
              <span className={`step-r ${s2Right.cls}`}>{s2Right.text}</span>
            </div>
            {n === 0 ? <div className="step-warn-box">Add a wallet below and confirm it's yours.</div> : null}

            {rows.map((r) => (
              <div className="step-wallet" key={r.key}>
                <span><span className="c">{r.label}</span>{r.addr ? <>&nbsp; {r.addr}</> : null}</span>
                {/* A cached row while the count can't be read: never green (B19 edge case). */}
                <span className={n === null ? "s checking" : "s"}>{n === null ? "Checking…" : "Covered ✓"}</span>
              </div>
            ))}
            {unnamed > 0 ? (
              <div className="step-hint">
                <button className="step-link inline" disabled={tailsBusy} onClick={showAddresses}>
                  {tailsBusy ? "Waiting for your signature..." : "Show addresses"}
                </button>
              </div>
            ) : null}
            {tailsMsg ? <div className="step-hint">{tailsMsg}</div> : null}

            {pending ? (
              <div className="step-pending">
                <div className="k">Confirm this {chainLabel(pending.chain)} wallet is yours</div>
                Open <b>{shortAddr(pending.wallet)}</b> in your wallet app and send exactly <b>{pending.amount} {pending.asset}</b> to that same address, then press Sent. A send from any other wallet won't count.
                {pending.expires_at ? ` Expires ${new Date(pending.expires_at * 1000).toLocaleTimeString()}.` : ""}
                <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={walletBusy} onClick={confirmSent}>
                  {walletBusy ? "Checking..." : "Sent"}
                </button>
                <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={walletBusy}
                  onClick={() => setPending(null)}>
                  Cancel
                </button>
              </div>
            ) : full || n === null ? null : n === 0 || addOpen ? (
              addForm
            ) : (
              <div className="step-hint">
                <button className="step-link inline" onClick={() => setAddOpen(true)}>Add another wallet</button>
              </div>
            )}
            {walletMsg ? (
              <div className="tx-status" style={{ marginTop: 8 }}>
                <div>{walletMsg}</div>
              </div>
            ) : null}
          </div>
        )}
      </div>

      <div>
        <div className="step-stats">
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
          <div className="side-stat">
            <div className="k">Yield so far</div>
            <div className="v">+{((Number(yieldIndex) / Number(YIELD_INDEX_PRECISION) - 1) * 100).toFixed(2)}%</div>
          </div>
        </div>
      </div>
    </div>
  );
}

function shortAddr(a: string): string {
  return a.length > 10 ? `${a.slice(0, 4)}…${a.slice(-4)}` : a;
}

// The safu-account is shown shortened (a full 56-char address breaks the phone layout, B18),
// so this copies the full address. Only for the safu-account: covered wallets are never exposed.
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(t);
  }, [copied]);
  const copy = () => navigator.clipboard?.writeText(text).then(() => setCopied(true), () => {});
  return (
    <button type="button" className={copied ? "copy-btn copied" : "copy-btn"} onClick={copy}
      aria-label={copied ? "Copied" : "Copy full safu-account address"} title={copied ? "Copied" : "Copy address"}>
      {copied ? (
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
      ) : (
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3" /></svg>
      )}
      {copied ? <span className="copy-label">Copied</span> : null}
    </button>
  );
}

// Last-4 list, cached per staker in this browser after the one signature (display only).
function tailsKey(staker: string): string {
  return `safu_covered_tails:${staker}`;
}

function readCachedTails(staker: string): CoveredWalletTail[] | null {
  try {
    const raw = localStorage.getItem(tailsKey(staker));
    return raw ? (JSON.parse(raw) as CoveredWalletTail[]) : null;
  } catch {
    return null;
  }
}

function cacheTails(staker: string, t: CoveredWalletTail[]): void {
  try {
    localStorage.setItem(tailsKey(staker), JSON.stringify(t));
  } catch {
    // Display cache only.
  }
}

/** The pool holds a stake for this account. Re-read a few times: the RPC can lag the relay's tx. */
async function stakeLanded(client: AppClient, account: string): Promise<boolean> {
  for (let i = 0; i < 3; i++) {
    const s = await readMyStake(client, account).catch(() => null);
    if (s && s.amount > 0n) return true;
    await new Promise((r) => setTimeout(r, 3000));
  }
  return false;
}
