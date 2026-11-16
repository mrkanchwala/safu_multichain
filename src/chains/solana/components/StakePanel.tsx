import { useEffect, useState } from "react";
import { useAction, useClient } from "../lib/client";
import { claimYield, stake, withdraw } from "../lib/actions";
import { addCoveredWallet, confirmCoveredWallet } from "../lib/claimApi";
import type { CoverChain, CoveredWalletResponse } from "../lib/claimApi";
import { num, setting } from "../lib/pool";
import { unstakeFeeBps } from "../lib/program";
import { fmtSol, parseSol, readCoveredCount, readCoveredWallets, readMyStake, refreshSoon, rememberCoveredWallet, stakeBounds, withdrawCheck } from "../lib/reads";
import type { CoveredWallet, MyStake } from "../lib/reads";
import { usePool } from "../lib/usePool";
import { toFriendlyError } from "../lib/friendly-error";
import { TxStatus } from "./TxStatus";
import {
  AddAnotherLink, AddWalletForm, CoverFoldedStep, CoverLockedStep, CoverOpenStep, ProofBox, StakeDoneStep, StakeManageStep,
  StakeOpenStep, StepStats, WalletMsg, coverRows, coverStatus, shortAddr,
} from "../../../shell/components/StakeSteps";

// Stake tab. SOL goes into the pool, which deploys most of it to Marinade (mSOL) for yield. Payouts
// (withdrawal, yield, claim payouts) go back to the staking wallet. Covered wallets are separate
// wallets proven by a self-transfer; the staking wallet itself can never be one (burn-wallet staking,
// same rule as the multichain site, enforced by the claim API too).
const MAX_COVERED_WALLETS = Number(num("MAX_COVERED_WALLETS"));
const WALLETS_TEXT = MAX_COVERED_WALLETS === 1 ? "1 wallet" : `${MAX_COVERED_WALLETS} wallets`;
// Covered wallets can be on Solana or Ethereum, any mix. The API checks the same address formats.
const CHAINS: { id: CoverChain; label: string; placeholder: string; valid: (a: string) => boolean }[] = [
  { id: "solana", label: "Solana", placeholder: "Solana wallet address", valid: (a) => /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(a) },
  { id: "sepolia", label: "Ethereum (Sepolia)", placeholder: "Ethereum address (0x...)", valid: (a) => /^0x[0-9a-fA-F]{40}$/.test(a) },
];
const chainLabel = (id?: string) => CHAINS.find((c) => c.id === (id ?? "solana"))?.label ?? id ?? "Solana";
const RATIOS = [num("TIER_A_RATIO"), num("TIER_B_RATIO"), num("TIER_C_RATIO")];

export function FeeNote({ bps }: { bps: number | null }) {
  if (bps === null) return null;
  return (
    <div className="ramp-disclosure" style={{ marginTop: 8 }}>
      If the pool has to unstake mSOL to pay you, Marinade's fee on that part comes out of the payment
      (about {(bps / 100).toFixed(2)}% right now). Usually the pool has enough SOL on hand and there is no fee.
    </div>
  );
}

export function StakePanel() {
  const client = useClient();
  const staker = client.address;
  const [refreshKey, setRefreshKey] = useState(0);
  const { pool } = usePool(refreshKey);
  const [myStake, setMyStake] = useState<MyStake | null>(null);
  const [feeBps, setFeeBps] = useState<number | null>(null);
  const [coveredWallets, setCoveredWallets] = useState<CoveredWallet[]>([]);

  const [stakeAmount, setStakeAmount] = useState("");
  const [withdrawAmount, setWithdrawAmount] = useState("");
  const [newWallet, setNewWallet] = useState("");
  const [newChain, setNewChain] = useState<CoverChain>("solana");
  const [pending, setPending] = useState<CoveredWalletResponse | null>(null);
  const [walletMsg, setWalletMsg] = useState<string | null>(null);
  const [walletBusy, setWalletBusy] = useState(false);
  // The program's own count (any browser); null = still checking or the read failed, which never shows green.
  const [coveredCount, setCoveredCount] = useState<number | null>(null);
  const [manageOpen, setManageOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setCoveredCount(null);
    if (staker) {
      readCoveredCount(staker)
        .then((c) => !cancelled && setCoveredCount(c))
        .catch(() => !cancelled && setCoveredCount(null));
    }
    return () => {
      cancelled = true;
    };
  }, [staker, refreshKey]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!pool || !staker) {
        setMyStake(null);
        setCoveredWallets(staker ? readCoveredWallets(staker) : []);
        return;
      }
      const s = await readMyStake(pool, staker).catch(() => null);
      const fee = s && s.amount > 0n ? await unstakeFeeBps(pool, s.amount + s.yieldOwed).catch(() => null) : null;
      if (!cancelled) {
        setMyStake(s);
        setFeeBps(fee);
        setCoveredWallets(readCoveredWallets(staker));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pool, staker]);

  const bounds = pool ? stakeBounds(pool) : null;
  const lamports = parseSol(stakeAmount);
  const amountValid = !!bounds && lamports !== null && lamports >= bounds.min && lamports <= bounds.max;
  const live = !!myStake && !myStake.forfeited;
  const done = () => refreshSoon(() => setRefreshKey((k) => k + 1));

  // Clear the amount only once it went through: a declined or failed send keeps it for a retry.
  const stakeAction = useAction(() => stake(client, pool, lamports ?? 0n).then((r) => (setStakeAmount(""), r)).finally(done));
  // Blank = the whole stake. A part must leave at least the min stake (checked here and on-chain).
  const withdrawLamports = withdrawAmount.trim() === "" ? (myStake?.amount ?? 0n) : parseSol(withdrawAmount);
  const withdrawProblem =
    live && pool ? (withdrawLamports === null ? "Enter an amount in SOL." : withdrawCheck(pool, myStake.amount, withdrawLamports)) : null;
  const withdrawAction = useAction(() => withdraw(client, pool, withdrawLamports ?? 0n).then((r) => (setWithdrawAmount(""), r)).finally(done));
  const takeYieldAction = useAction(() => claimYield(client, pool).finally(done));

  function registered(r: CoveredWalletResponse) {
    if (!staker) return;
    const row = rememberCoveredWallet(staker, r.chain, r.wallet, r.registered_at ?? Math.floor(Date.now() / 1000));
    setCoveredWallets((cur) => (cur.some((w) => (w.chain ?? "solana") === row.chain && w.wallet === row.wallet) ? cur : [...cur, row]));
    setPending(null);
    setNewWallet("");
    setWalletMsg(r.onchain && r.onchain !== "written" ? "Registered. The on-chain copy will follow shortly." : "Registered.");
    setAddOpen(false);
    done();
  }

  async function addWallet() {
    setWalletMsg(null);
    if (!staker) return setWalletMsg("Connect a wallet first.");
    if (!live) return setWalletMsg("Stake first, then add the wallets you want covered.");
    if (coveredWallets.length >= MAX_COVERED_WALLETS) return setWalletMsg(`You can cover at most ${WALLETS_TEXT}.`);
    const w = newWallet.trim();
    if (!w) return setWalletMsg("Enter a wallet address.");
    const chain = CHAINS.find((c) => c.id === newChain)!;
    if (!chain.valid(w)) return setWalletMsg(`That isn't a valid ${chain.label} address. Check it and try again.`);
    if (w === staker) return setWalletMsg("A covered wallet can't be your staking wallet. Cover the wallets that hold your money.");
    setWalletBusy(true);
    try {
      const r = await addCoveredWallet(client, staker, newChain, w);
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

  const n = coveredCount;
  const protectedNow = live && n !== null && n > 0;
  const full = n !== null && n >= MAX_COVERED_WALLETS;
  const status = coverStatus(live, n, MAX_COVERED_WALLETS);
  const { rows } = coverRows(
    coveredWallets.map((w) => ({ key: `${w.chain ?? "solana"}-${w.wallet}`, label: chainLabel(w.chain), addr: shortAddr(w.wallet) })),
    n,
  );

  return (
    <div className="panel">
      <div>
        {!myStake ? (
          <StakeOpenStep
            range={bounds ? `${fmtSol(bounds.min, 2)}–${fmtSol(bounds.max, 2)} SOL` : " "}
            amount={stakeAmount} setAmount={setStakeAmount} unit="SOL"
            canStake={!!staker && amountValid} running={stakeAction.isRunning}
            label="Stake" onStake={() => stakeAction.dispatch()} status={<TxStatus action={stakeAction} />}
          />
        ) : !manageOpen ? (
          <StakeDoneStep
            title={myStake.forfeited ? "Stake used to pay a claim" : `Staked ${fmtSol(myStake.amount)} SOL`}
            onManage={() => setManageOpen(true)}
          />
        ) : (
          <StakeManageStep
            onClose={() => setManageOpen(false)}
            rows={[
              { k: "Staked", v: `${fmtSol(myStake.amount)} SOL` },
              { k: "Yield earned", v: `${fmtSol(myStake.yieldOwed, 6)} SOL` },
              ...(myStake.forfeited ? [{ k: "Status", v: "Used to pay a claim" }] : []),
            ]}
          >
            {live ? (
              <>
                <div className="field-group" style={{ marginTop: 8 }}>
                  <div className="field-label">
                    <span>Withdraw</span>
                    <span>blank = all of it</span>
                  </div>
                  <div className="field-input">
                    <input placeholder={fmtSol(myStake.amount)} value={withdrawAmount} onChange={(e) => setWithdrawAmount(e.target.value)} inputMode="decimal" aria-label="Amount to withdraw in SOL" />
                    <span className="unit">SOL</span>
                  </div>
                  {withdrawAmount.trim() !== "" && withdrawProblem ? <div className="ramp-disclosure">{withdrawProblem}</div> : null}
                </div>
                {myStake.yieldOwed > 0n ? (
                  <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={takeYieldAction.isRunning} onClick={() => takeYieldAction.dispatch()}>
                    {takeYieldAction.isRunning ? "Sending..." : `Take ${fmtSol(myStake.yieldOwed, 6)} SOL yield, keep my stake`}
                  </button>
                ) : null}
                <TxStatus action={takeYieldAction} />
                <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={!!withdrawProblem || withdrawAction.isRunning} onClick={() => withdrawAction.dispatch()}>
                  {withdrawAction.isRunning ? "Withdrawing..." : withdrawAmount.trim() === "" ? "Withdraw all + yield" : "Withdraw this + all yield"}
                </button>
                <TxStatus action={withdrawAction} />
                <FeeNote bps={feeBps} />
                <div className="step-warn">
                  Withdrawing all of it ends cover for your wallets; a part shrinks your cover with it. Money open claims still
                  need stays in the pool until they're paid.
                </div>
              </>
            ) : null}
          </StakeManageStep>
        )}

        {myStake && manageOpen ? (
          <CoverFoldedStep done={protectedNow} status={status} />
        ) : !live ? (
          <CoverLockedStep status={status} ended={!myStake && n !== null && n > 0} />
        ) : (
          <CoverOpenStep done={protectedNow} status={status} count={n} rows={rows}>
            {pending ? (
              <ProofBox chain={chainLabel(pending.chain)} wallet={pending.wallet} amount={pending.amount} asset={pending.asset}
                expiresAt={pending.expires_at} busy={walletBusy} onSent={confirmSent} onCancel={() => setPending(null)} />
            ) : full || n === null ? null : n === 0 || addOpen ? (
              <AddWalletForm chains={CHAINS} chain={newChain} setChain={(c) => (setNewChain(c), setWalletMsg(null))}
                value={newWallet} setValue={setNewWallet} placeholder={CHAINS.find((c) => c.id === newChain)!.placeholder}
                busy={walletBusy} onAdd={addWallet} max={MAX_COVERED_WALLETS} where="on Solana or Ethereum" />
            ) : (
              <AddAnotherLink onClick={() => setAddOpen(true)} />
            )}
            <div className="step-hint">A loss on Ethereum is paid in SOL at the SOL price when the theft was detected.</div>
            <WalletMsg text={walletMsg} />
          </CoverOpenStep>
        )}
      </div>

      <div>
        <StepStats items={[
          { k: "Pool cap", v: `${pool ? fmtSol(setting(pool, "POOL_CAP"), 0) : "--"} SOL` },
          { k: "Total staked", v: `${pool ? fmtSol(pool.total_staked, 2) : "--"} SOL` },
          { k: "Stakers", v: pool ? pool.total_stakers.toString() : "--" },
          { k: "Cover", v: `Up to ${RATIOS[0].toString()}x` },
        ]} />
      </div>
    </div>
  );
}
