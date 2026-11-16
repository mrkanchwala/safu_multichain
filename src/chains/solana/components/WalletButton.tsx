import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { AppClient } from "../lib/client";
import { solanaWallets } from "../lib/client";
import { toFriendlyError } from "../lib/friendly-error";
import { POOL } from "../lib/pool";
import { SolanaIcon } from "../../../shell/components/Icons";

// One Connect button: the Solana wallets found in this browser. On a phone, open the site in the
// wallet app's own browser (Phantom, Solflare), which shows up here the same way.

function short(addr: string): string {
  return `${addr.slice(0, 4)}...${addr.slice(-4)}`;
}

function WalletRow({ icon, name, sub, onClick }: { icon: ReactNode; name: string; sub?: string; onClick: () => void }) {
  return (
    <button className="wallet-row" onClick={onClick}>
      <span className="wallet-row-icon">{icon}</span>
      <span className="wallet-row-text">
        <span className="wallet-row-name">{name}</span>
        {sub ? <span className="wallet-row-sub">{sub}</span> : null}
      </span>
    </button>
  );
}

export function WalletButton({ client }: { client: AppClient }) {
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Escape.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (client.address) {
    return (
      <div className="wallet-menu">
        <button
          className="connect-btn"
          onClick={() => client.disconnect()}
          aria-label={`Disconnect wallet ${client.address}`}
          title="Disconnect"
        >
          {short(client.address)}
          <span className="disc-word"> · Disconnect</span>
          <span className="disc-x" aria-hidden="true"> ✕</span>
        </button>
      </div>
    );
  }

  async function go(id?: string) {
    setError(null);
    setOpen(false);
    try {
      await client.connect(id);
    } catch (e) {
      const friendly = toFriendlyError(e);
      if (!friendly.cancelled) {
        setError(friendly.message);
        setOpen(true);
      }
    }
  }

  const wallets = solanaWallets();

  return (
    <div className="wallet-menu" ref={menuRef}>
      <button className="connect-btn" disabled={client.connecting} onClick={() => { setError(null); setOpen((v) => !v); }} aria-expanded={open}>
        {client.connecting ? "Connecting..." : "Connect Wallet"}
      </button>
      {open ? (
        <div className="wallet-dropdown" role="menu">
          <div className="wallet-head">
            Solana <span className="wallet-net">{POOL.cluster === "devnet" ? "Devnet" : "Local"}</span>
          </div>
          {wallets.length ? (
            wallets.map((w) => (
              <WalletRow key={w.id} name={w.name} icon={w.icon ? <img src={w.icon} alt="" /> : <SolanaIcon />} onClick={() => go(w.id)} />
            ))
          ) : (
            <div className="wallet-empty">No Solana wallet found. On a phone, open this page in your wallet app's browser.</div>
          )}
          {error ? <div className="wallet-error">{error}</div> : null}
        </div>
      ) : null}
    </div>
  );
}
