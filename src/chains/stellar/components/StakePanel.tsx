import { useEffect, useState } from "react";
import { POOL } from "../lib/pool";
import { useAction, useClient } from "../lib/client";
import type { AppClient } from "../lib/client";
import { claimStakerYield, stake, withdrawStake } from "../lib/actions";
import { addCoveredWallet, confirmCoveredWallet, countCoveredWallets, listMyCoveredWallets } from "../lib/claimApi";
import { walletChainMismatch } from "../lib/wallet-chain";
import type { CoveredWalletResponse, CoveredWalletTail } from "../lib/claimApi";
import {
  AddAnotherLink, AddWalletForm, CopyButton, CoverFoldedStep, CoverLockedStep, CoverOpenStep, ProofBox, StakeDoneStep,
  StakeManageStep, StakeOpenStep, StepStats, WalletMsg, coverRows, coverStatus, shortAddr,
} from "../../../shell/components/StakeSteps";
import type { CoverRow } from "../../../shell/components/StakeSteps";
import {
  evmBurnToAdapter,
  evmUsdcBalance,
  finishOnStellar,
  relayHomeAction,
  burnForStake,
  solanaBurnToAdapter,
  solanaUsdcBalance,
} from "../lib/crosschain";
import {
  fmtExact, fmtUsdc, fmtYield, readCoveredWallets, readMyBacking, readMyStake, readPoolStats, readStakeBounds, readYieldIndex,
  rememberCoveredWallet,
} from "../lib/reads";
import type { CoveredWallet, MyStake } from "../lib/reads";
import { CHAINS, MIN_BRIDGEABLE_RAW, YIELD_INDEX_PRECISION } from "../lib/network";
import { useStakeBounds } from "../lib/useStakeBounds";
import type { ChainId } from "../lib/network";
import { useStaker } from "../lib/useStaker";
import { useTick } from "../../../shell/lib/useTick";
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
  const bounds = useStakeBounds();

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

  const amountValid = !!bounds && Number(stakeAmount) >= bounds.min && Number(stakeAmount) <= bounds.max;

  const stakeAction = useAction(async (signal) => {
    if (!client.address) throw new Error("Connect a wallet first.");
    if (!staker) throw new Error("Still loading your account. Try again in a moment.");
    // Read the stake fresh before sending anything (2026-10-31: a Solana user staked 3 times; the
    // pool took one, the other two waited in the safu-account). A failed read stops here too:
    // never send money on a guess.
    const before = await depositSnapshot(client, staker).catch(() => null);
    if (!before) throw new Error("Couldn't check your stake right now. Try again in a moment.");
    if (before.stake > 0n) {
      throw new Error("You already have a stake. A wallet holds one stake: withdraw it first to stake a new amount.");
    }
    try {
      if (client.kind === "stellar") {
        // Beneficiary == staker (stake.rs, 2026-09-19): payouts land back on the wallet that staked.
        return await stake(client, null, stakeAmount, client.address);
      }
      const kind = client.kind as "evm" | "solana";
      const usdc6 = BigInt(Math.round(Number(stakeAmount) * 1e6));
      // Circle's fee comes out of the burn, so it is sent on top: the typed amount is what gets staked.
      // Fresh limits for the send itself: never burn on a cached or guessed number.
      const live = await readStakeBounds();
      const burn6 = burnForStake(usdc6, BigInt(Math.round(live.min * 1e6)), BigInt(Math.round(live.max * 1e6)));
      const sending = String(Number(burn6) / 1e6);
      if (homeUsdc !== null && burn6 > homeUsdc) {
        throw new Error(`You don't have enough USDC in this wallet. Staking ${stakeAmount} USDC sends ${sending}, including Circle's fee.`);
      }
      setProgress(`Sending ${sending} USDC on ${kind === "evm" ? POOL.evm.label : POOL.solana.label} (${stakeAmount} to stake + Circle's fee)...`);
      const burn = kind === "evm"
        ? await evmBurnToAdapter(client, burn6, "stake", setProgress)
        : await solanaBurnToAdapter(client, burn6, "stake");
      const account = await finishOnStellar("stake", kind, burn, setProgress, signal);
      // A deposit the pool refuses (already staked, full, paused) waits in the safu-account
      // (cctp-adapter). Say where THIS deposit went by comparing before and after, never by
      // "a stake exists". New pool: above the max stake, the rest becomes backing (stake.rs A7).
      const out = await depositOutcome(client, account, before);
      if (out.staked <= 0n && out.backed <= 0n) {
        throw new Error("Your USDC reached your safu-account but wasn't staked. It's safe there. Message the SAFU team and we'll help you finish it.");
      }
      const parts = [
        out.staked > 0n ? `Staked ${fmtUsdc(out.staked)} USDC` : null,
        out.backed > 0n ? `${fmtUsdc(out.backed)} USDC went to backing` : null,
      ].filter(Boolean);
      return `${parts.join(", ")} from your safu-account ${account.slice(0, 6)}...`;
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
  const full = n !== null && n >= MAX_COVERED_WALLETS;
  const status = coverStatus(staked, n, MAX_COVERED_WALLETS);

  // Wallets this browser added (full address known), then the server's last-4 list for the rest.
  const known: CoverRow[] = coveredWallets.map((w, i) => ({
    key: `l-${w.chain}-${w.wallet}-${i}`, label: chainLabel(w.chain), addr: shortAddr(w.wallet),
  }));
  for (const t of tails ?? []) {
    if (!coveredWallets.some((w) => w.chain === t.chain && w.wallet.endsWith(t.tail))) {
      known.push({ key: `t-${t.chain}-${t.tail}`, label: chainLabel(t.chain), addr: `…${t.tail}` });
    }
  }
  const { rows, unnamed } = coverRows(known, n);

  return (
    <div className="panel">
      <div>
        {!staked ? (
          <StakeOpenStep
            range={bounds ? `$${bounds.min}–$${bounds.max.toLocaleString()}` : " "}
            amount={stakeAmount} setAmount={setStakeAmount} unit="USDC"
            canStake={!!client.address && amountValid} running={stakeAction.isRunning}
            label={crossChain ? "Stake via Circle" : "Stake"} onStake={() => stakeAction.dispatch()}
            status={<>{progress ? <div className="ramp-disclosure" style={{ marginTop: 8 }}>{progress}</div> : null}<TxStatus action={stakeAction} /></>}
          >
            {crossChain ? (
              <div className="step-hint">
                Your USDC crosses to Stellar on Circle's bridge and your own safu-account stakes it.
                {homeUsdc !== null ? ` USDC in your wallet: ${(Number(homeUsdc) / 1e6).toFixed(2)}.` : ""}
              </div>
            ) : null}
          </StakeOpenStep>
        ) : !manageOpen ? (
          <StakeDoneStep title={`Staked ${fmtUsdc(myStake.amount)} USDC`} onManage={() => setManageOpen(true)} />
        ) : (
          <StakeManageStep
            onClose={() => setManageOpen(false)}
            rows={[
              { k: "Staked", v: `${fmtUsdc(myStake.amount)} USDC` },
              { k: "Withdrawable now", v: `${fmtExact(myStake.withdrawable)} USDC (+${yieldStripPct(myStake.withdrawable, myStake.amount)}%)` },
              ...(staker && crossChain
                ? [{ k: "Your safu-account (Stellar)", v: <span className="addr-copy"><span title={staker}>{shortAddr(staker)}</span><CopyButton text={staker} /></span> }]
                : []),
            ]}
          >
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
          </StakeManageStep>
        )}

        {staked && manageOpen ? (
          <CoverFoldedStep done={protectedNow} status={status} />
        ) : !staked ? (
          <CoverLockedStep status={status} ended={n !== null && n > 0} />
        ) : (
          <CoverOpenStep done={protectedNow} status={status} count={n} rows={rows}>
            {unnamed > 0 ? (
              <div className="step-hint">
                <button className="step-link inline" disabled={tailsBusy} onClick={showAddresses}>
                  {tailsBusy ? "Waiting for your signature..." : "Show addresses"}
                </button>
              </div>
            ) : null}
            {tailsMsg ? <div className="step-hint">{tailsMsg}</div> : null}
            {pending ? (
              <ProofBox chain={chainLabel(pending.chain)} wallet={pending.wallet} amount={pending.amount} asset={pending.asset}
                expiresAt={pending.expires_at} busy={walletBusy} onSent={confirmSent} onCancel={() => setPending(null)} />
            ) : full || n === null ? null : n === 0 || addOpen ? (
              <AddWalletForm chains={CHAINS} chain={newChain} setChain={setNewChain} value={newWallet} setValue={setNewWallet}
                placeholder="Wallet address on that chain" busy={walletBusy} onAdd={addWallet} max={MAX_COVERED_WALLETS} where="on any chain" />
            ) : (
              <AddAnotherLink onClick={() => setAddOpen(true)} />
            )}
            <WalletMsg text={walletMsg} />
          </CoverOpenStep>
        )}
      </div>

      <div>
        <StepStats items={[
          { k: "Pool cap", v: bounds ? `$${bounds.cap.toLocaleString()}` : "--" },
          { k: "Total staked", v: `${fmtUsdc(poolStats.totalStaked)} USDC` },
          { k: "Stakers", v: poolStats.totalStakers },
          { k: "Yield so far", v: `+${((Number(yieldIndex) / Number(YIELD_INDEX_PRECISION) - 1) * 100).toFixed(2)}%` },
        ]} />
      </div>
    </div>
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

type DepositSnapshot = { stake: bigint; backing: bigint | null };

/** Stake and backing for this account. The stake read must succeed (it throws otherwise); a failed
 *  backing read is null, so it never blocks a stake or fakes a backing result. */
async function depositSnapshot(client: AppClient, account: string): Promise<DepositSnapshot> {
  const [s, b] = await Promise.all([
    readMyStake(client, account),
    readMyBacking(account).catch(() => undefined),
  ]);
  return {
    stake: s?.amount ?? 0n,
    backing: b === undefined ? null : b ? b.amount + b.pendingAmount : 0n,
  };
}

/** How much of THIS deposit the pool took as stake and as backing. Re-read a few times: the RPC
 *  can lag the relay's tx. */
async function depositOutcome(
  client: AppClient, account: string, before: DepositSnapshot,
): Promise<{ staked: bigint; backed: bigint }> {
  let out = { staked: 0n, backed: 0n };
  for (let i = 0; i < 3; i++) {
    const now = await depositSnapshot(client, account).catch(() => null);
    if (now) {
      out = {
        staked: now.stake - before.stake,
        backed: now.backing !== null && before.backing !== null ? now.backing - before.backing : 0n,
      };
      if (out.staked > 0n || out.backed > 0n) return out;
    }
    await new Promise((r) => setTimeout(r, 3000));
  }
  return out;
}
