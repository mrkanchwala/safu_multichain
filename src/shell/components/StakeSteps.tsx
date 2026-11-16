import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { FAQ_ANSWER, faqAnswerHref } from "../lib/links";

// The Stake tab's two steps, shared by every pool (B19 after the Tetra tester feedback, shared 16 Nov):
//   1 Stake: open until staked, then one line; "Manage stake" reopens it.
//   2 Cover your wallets: locked until staked; amber "Not protected yet" at 0, green once one is covered.
// Each pool's StakePanel owns its data and actions and passes them in; this file owns the layout and words.

/** First and last 4 characters: a full address breaks the phone layout (B18). */
export function shortAddr(a: string): string {
  return a.length > 10 ? `${a.slice(0, 4)}…${a.slice(-4)}` : a;
}

/** One covered-wallet row. `addr` empty = a wallet the count has but this browser can't name. */
export type CoverRow = { key: string; label: string; addr: string };

/** Rows from the wallets this browser knows, cut to the pool's own count (a stale cache must never show
 *  "Covered" under 0/N), then "Wallet N" rows for any the count has that nobody here can name. */
export function coverRows(known: CoverRow[], count: number | null): { rows: CoverRow[]; unnamed: number } {
  const rows = count !== null ? known.slice(0, count) : [...known];
  const unnamed = count !== null ? Math.max(0, count - rows.length) : 0;
  for (let i = 0; i < unnamed; i++) rows.push({ key: `u-${i}`, label: `Wallet ${rows.length + 1}`, addr: "" });
  return { rows, unnamed };
}

/** Step 2's right-hand status. Green needs an active stake AND a count the pool itself reports. */
export function coverStatus(staked: boolean, count: number | null, max: number): { text: string; cls: string } {
  if (!staked) return { text: `${count ?? 0}/${max}`, cls: "" };
  if (count === null) return { text: "Checking…", cls: "" };
  if (count === 0) return { text: `0/${max} · Not protected yet`, cls: "warn" };
  return { text: `${count}/${max} · Protected`, cls: "ok" };
}

export function StakeOpenStep({ range, amount, setAmount, unit, canStake, running, label, onStake, status, children }: {
  range: string; amount: string; setAmount: (v: string) => void; unit: string; canStake: boolean; running: boolean;
  label: string; onStake: () => void; status: ReactNode; children?: ReactNode;
}) {
  return (
    <div className="step active">
      <div className="step-h">
        <span className="step-n">1</span>Stake
        <span className="step-r">{range}</span>
      </div>
      <div className="field-input" style={{ marginTop: 12 }}>
        <input placeholder="0" value={amount} onChange={(e) => setAmount(e.target.value)} inputMode="decimal" aria-label={`Stake amount in ${unit}`} />
        <span className="unit">{unit}</span>
      </div>
      <button className="primary-action" disabled={!canStake || running} onClick={onStake}>
        {running ? "Staking..." : label}
      </button>
      {status}
      <div className="step-hint">
        Use a fresh wallet you keep safe. It gets your payouts but isn't covered itself.{" "}
        <a href={faqAnswerHref(FAQ_ANSWER.whichWallet)}>Which wallet?</a>
      </div>
      {children}
    </div>
  );
}

export function StakeDoneStep({ title, onManage }: { title: string; onManage: () => void }) {
  return (
    <div className="step done">
      <div className="step-h">
        <span className="step-n">✓</span>{title}
        <button className="step-link" onClick={onManage}>Manage stake</button>
      </div>
    </div>
  );
}

export function StakeManageStep({ rows, onClose, children }: {
  rows: { k: string; v: ReactNode }[]; onClose: () => void; children: ReactNode;
}) {
  return (
    <div className="step active">
      <div className="step-h">
        <span className="step-n">1</span>Your stake
        <button className="step-link" onClick={onClose}>Close ✕</button>
      </div>
      <div className="step-rows">
        {rows.map((r) => (
          <div className="step-row" key={r.k}><span className="k">{r.k}</span><span className="v">{r.v}</span></div>
        ))}
      </div>
      {children}
    </div>
  );
}

/** Step 2 while step 1 is open for managing: one line. */
export function CoverFoldedStep({ done, status }: { done: boolean; status: { text: string; cls: string } }) {
  return (
    <div className={`step${done ? " done" : ""}`}>
      <div className="step-h">
        <span className="step-n">{done ? "✓" : "2"}</span>Cover your wallets
        <span className={`step-r ${status.cls}`}>{status.text}</span>
      </div>
    </div>
  );
}

export function CoverLockedStep({ status, ended }: { status: { text: string; cls: string }; ended: boolean }) {
  return (
    <div className="step locked">
      <div className="step-h">
        <span className="step-n">2</span>Cover your wallets
        <span className="step-r">{status.text}</span>
      </div>
      <div className="step-hint">
        {ended ? "Cover ended when you withdrew. Stake again to bring it back." : "Stake first, then add the wallets you want covered."}
      </div>
    </div>
  );
}

export function CoverOpenStep({ done, status, count, rows, children }: {
  done: boolean; status: { text: string; cls: string }; count: number | null; rows: CoverRow[]; children: ReactNode;
}) {
  return (
    <div className={`step ${done ? "done" : "active"}`}>
      <div className="step-h">
        <span className="step-n">{done ? "✓" : "2"}</span>Cover your wallets
        <span className={`step-r ${status.cls}`}>{status.text}</span>
      </div>
      {count === 0 ? <div className="step-warn-box">Add a wallet below and confirm it's yours.</div> : null}
      {rows.map((r) => (
        <div className="step-wallet" key={r.key}>
          <span><span className="c">{r.label}</span>{r.addr ? <>&nbsp; {r.addr}</> : null}</span>
          {/* A cached row while the count can't be read: never green (B19 edge case). */}
          <span className={count === null ? "s checking" : "s"}>{count === null ? "Checking…" : "Covered ✓"}</span>
        </div>
      ))}
      {children}
    </div>
  );
}

/** The self-send that proves a covered wallet is yours. */
export function ProofBox({ chain, wallet, amount, asset, expiresAt, busy, onSent, onCancel }: {
  chain: string; wallet: string; amount?: string | null; asset?: string | null; expiresAt?: number | null; busy: boolean;
  onSent: () => void; onCancel: () => void;
}) {
  return (
    <div className="step-pending">
      <div className="k">Confirm this {chain} wallet is yours</div>
      Open <b>{shortAddr(wallet)}</b> in your wallet app and send exactly <b>{amount} {asset}</b> to that same address, then press Sent. A send from any other wallet won't count.
      {expiresAt ? ` Expires ${new Date(expiresAt * 1000).toLocaleTimeString()}.` : ""}
      <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={busy} onClick={onSent}>
        {busy ? "Checking..." : "Sent"}
      </button>
      <button className="secondary-action" style={{ width: "100%", marginTop: 8 }} disabled={busy} onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}

export function AddWalletForm<C extends string>({ chains, chain, setChain, value, setValue, placeholder, busy, onAdd, max, where }: {
  chains: readonly { id: C; label: string }[]; chain: C; setChain: (c: C) => void; value: string; setValue: (v: string) => void;
  placeholder: string; busy: boolean; onAdd: () => void; max: number; where: string;
}) {
  return (
    <>
      <div className="ramp-toggle" role="group" aria-label="Chain of the wallet to cover" style={{ marginTop: 10, marginBottom: 8 }}>
        {chains.map((c) => (
          <button key={c.id} className={`ramp-toggle-btn${chain === c.id ? " active" : ""}`} aria-pressed={chain === c.id} onClick={() => setChain(c.id)}>
            {c.label}
          </button>
        ))}
      </div>
      <div className="field-input">
        <input className="cover-addr" placeholder={placeholder} value={value} onChange={(e) => setValue(e.target.value)} aria-label="Wallet to cover" />
      </div>
      <button className="secondary-action" style={{ width: "100%" }} disabled={busy} onClick={onAdd}>
        {busy ? "Checking..." : "Add covered wallet"}
      </button>
      <div className="step-hint">
        Up to {max} wallets {where}. Each one sends a tiny amount to itself to prove it's yours.
        Add them before anything happens.{" "}
        <a href={faqAnswerHref(FAQ_ANSWER.addCovered)}>What's a covered wallet?</a>
      </div>
    </>
  );
}

export function AddAnotherLink({ onClick }: { onClick: () => void }) {
  return (
    <div className="step-hint">
      <button className="step-link inline" onClick={onClick}>Add another wallet</button>
    </div>
  );
}

export function WalletMsg({ text }: { text: string | null }) {
  if (!text) return null;
  return (
    <div className="tx-status" style={{ marginTop: 8 }}>
      <div>{text}</div>
    </div>
  );
}

export function StepStats({ items }: { items: { k: string; v: ReactNode }[] }) {
  return (
    <div className="step-stats">
      {items.map((it) => (
        <div className="side-stat" key={it.k}>
          <div className="k">{it.k}</div>
          <div className="v">{it.v}</div>
        </div>
      ))}
    </div>
  );
}

// Shown shortened (B18), so this copies the full address. Only for the staker's own account:
// covered wallets are never exposed.
export function CopyButton({ text }: { text: string }) {
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
