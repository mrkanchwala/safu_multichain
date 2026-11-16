import { useEffect, useState } from "react";
import { useAction, useClient } from "../lib/client";
import { claimYield, stake, withdraw } from "../lib/actions";
import { addCoveredWallet, confirmCoveredWallet } from "../lib/claimApi";
import type { CoverChain, CoveredWalletResponse } from "../lib/claimApi";
import { num, setting } from "../lib/pool";
import { unstakeFeeBps } from "../lib/program";
import { fmtSol, parseSol, readCoveredWallets, readMyStake, refreshSoon, rememberCoveredWallet, stakeBounds, withdrawCheck } from "../lib/reads";
import type { CoveredWallet, MyStake } from "../lib/reads";
import { usePool } from "../lib/usePool";
import { toFriendlyError } from "../lib/friendly-error";
import { TxStatus } from "./TxStatus";

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

  return (
    <div className="panel">
      <div>
        <div className="field-group">
          <div className="field-label">
            <span>Stake</span>
            <span>{bounds ? `${fmtSol(bounds.min, 2)}–${fmtSol(bounds.max, 2)} SOL` : " "}</span>
          </div>
          <div className="field-input">
            <input placeholder="0" value={stakeAmount} onChange={(e) => setStakeAmount(e.target.value)} inputMode="decimal" aria-label="Stake amount in SOL" />
            <span className="unit">SOL</span>
          </div>
        </div>
        <button className="primary-action" disabled={!staker || !amountValid || !!myStake || stakeAction.isRunning} onClick={() => stakeAction.dispatch()}>
          {stakeAction.isRunning ? "Staking..." : "Stake"}
        </button>
        <TxStatus action={stakeAction} />
        <div className="ramp-disclosure" style={{ marginTop: 8 }}>
          Payouts go back to this same wallet. Stake from a separate wallet you're comfortable connecting
          here, and cover the wallets that hold your money below, without ever connecting them.
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
                <div className="k">Principal (coverage is based on this)</div>
                <div className="v">{fmtSol(myStake.amount)} SOL</div>
              </div>
              <div className="side-stat">
                <div className="k">Yield earned</div>
                <div className="v">{fmtSol(myStake.yieldOwed, 6)} SOL</div>
              </div>
              {myStake.forfeited ? (
                <div className="side-stat">
                  <div className="k">Status</div>
                  <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 14 }}>Used to pay a claim</div>
                </div>
              ) : null}
            </>
          )}
        </div>
        {live ? (
          <div className="field-group">
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
        ) : null}
        <button className="secondary-action" style={{ width: "100%" }} disabled={!live || !!withdrawProblem || withdrawAction.isRunning} onClick={() => withdrawAction.dispatch()}>
          {withdrawAction.isRunning ? "Withdrawing..." : withdrawAmount.trim() === "" ? "Withdraw all + yield" : "Withdraw this + all yield"}
        </button>
        {live ? (
          <div className="ramp-disclosure" style={{ marginTop: 8 }}>
            You can take out part of your stake; your coverage shrinks with it. Money open claims still need
            stays in the pool until they're paid, so a withdrawal can wait. It's never lost.
          </div>
        ) : null}
        <TxStatus action={withdrawAction} />
        {live && myStake.yieldOwed > 0n ? (
          <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={takeYieldAction.isRunning} onClick={() => takeYieldAction.dispatch()}>
            {takeYieldAction.isRunning ? "Sending..." : `Take ${fmtSol(myStake.yieldOwed, 6)} SOL yield, keep my stake`}
          </button>
        ) : null}
        <TxStatus action={takeYieldAction} />
        {live ? <FeeNote bps={feeBps} /> : null}

        <div className="field-group" style={{ marginTop: 20 }}>
          <div className="field-label">
            <span>Covered wallets</span>
            <span>{coveredWallets.length}/{MAX_COVERED_WALLETS}</span>
          </div>
          <div className="ramp-disclosure">
            Register a wallet before crypto is stolen from it, because a claim can only name a wallet already on this list.
            You can cover {WALLETS_TEXT}, Solana or Ethereum in any mix, not your staking wallet. To prove a
            wallet is yours, it sends a tiny amount to itself on its own chain. A covered wallet can't be removed
            later, and a loss on Ethereum is paid in SOL at the SOL price when the theft was detected.
          </div>
          {coveredWallets.map((w) => (
            <div className="side-stat" key={`${w.chain ?? "solana"}-${w.wallet}`}>
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
              <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={walletBusy} onClick={() => setPending(null)}>
                Cancel
              </button>
            </div>
          ) : coveredWallets.length < MAX_COVERED_WALLETS ? (
            <>
              <div className="ramp-toggle" role="group" aria-label="Chain of the wallet to cover" style={{ marginTop: 10, marginBottom: 8 }}>
                {CHAINS.map((c) => (
                  <button key={c.id} className={`ramp-toggle-btn${newChain === c.id ? " active" : ""}`} aria-pressed={newChain === c.id}
                    onClick={() => (setNewChain(c.id), setWalletMsg(null))}>
                    {c.label}
                  </button>
                ))}
              </div>
              <div className="field-input">
                <input placeholder={CHAINS.find((c) => c.id === newChain)!.placeholder} value={newWallet}
                  onChange={(e) => setNewWallet(e.target.value)} aria-label="Wallet to cover" />
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
          <div className="v">{pool ? fmtSol(setting(pool, "POOL_CAP"), 0) : "--"} SOL</div>
        </div>
        <div className="side-stat">
          <div className="k">Total staked</div>
          <div className="v">{pool ? fmtSol(pool.total_staked, 2) : "--"} SOL</div>
        </div>
        <div className="side-stat">
          <div className="k">Stakers</div>
          <div className="v">{pool ? pool.total_stakers.toString() : "--"}</div>
        </div>
        <div className="side-stat">
          <div className="k">Coverage ceilings</div>
          <div className="v" style={{ fontFamily: "var(--font-body)", fontSize: 13 }}>
            Up to {RATIOS[0].toString()}x (Tier A) &middot; up to {RATIOS[1].toString()}x (Tier B) &middot; up to{" "}
            {RATIOS[2].toString()}x (Tier C) of your principal, never more.
          </div>
        </div>
      </div>
    </div>
  );
}
