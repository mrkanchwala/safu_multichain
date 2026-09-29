import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { AppClient, WalletKind } from "../lib/client";
import { WALLETCONNECT_ID, evmWallets, solanaWallets } from "../lib/client";
import { toFriendlyError } from "../lib/friendly-error";
import { EthereumIcon, SolanaIcon, StellarIcon, WalletConnectIcon } from "./Icons";
import { POOL } from "../lib/pool";

// One Connect button, two steps (redesigned 2026-09-25): step 1 picks the chain, step 2
// lists that chain's wallets. Stellar skips step 2: the wallet kit's own window already lists its
// extensions + WalletConnect. Ethereum lists EIP-6963 browser wallets, Solana wallet-standard
// ones; both end with WalletConnect (phone).

const KIND_LABEL: Record<WalletKind, string> = { stellar: "Stellar", evm: "Ethereum", solana: "Solana" };

const CHAINS: { kind: WalletKind; name: string; net: string; icon: ReactNode }[] = [
  { kind: "stellar", name: "Stellar", net: POOL.stellar.short, icon: <StellarIcon /> },
  { kind: "evm", name: "Ethereum", net: POOL.evm.short, icon: <EthereumIcon /> },
  { kind: "solana", name: "Solana", net: POOL.solana.short, icon: <SolanaIcon /> },
];

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
  const [chain, setChain] = useState<WalletKind | null>(null);
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

  if (client.address && client.kind) {
    return (
      <div className="wallet-menu">
        <button className="connect-btn" onClick={() => client.disconnect()}>
          {KIND_LABEL[client.kind]} · {short(client.address)} · Disconnect
        </button>
      </div>
    );
  }

  function toggle() {
    setError(null);
    setChain(null);
    setOpen((v) => !v);
  }

  async function go(kind: WalletKind, id?: string) {
    setError(null);
    setOpen(false);
    try {
      await client.connect(kind, id);
    } catch (e) {
      const friendly = toFriendlyError(e);
      if (!friendly.cancelled) {
        setError(friendly.message);
        setChain(kind === "stellar" ? null : kind);
        setOpen(true);
      }
    }
  }

  function pickChain(kind: WalletKind) {
    setError(null);
    if (kind === "stellar") void go("stellar");
    else setChain(kind);
  }

  const current = CHAINS.find((c) => c.kind === chain);
  const wallets =
    chain === "evm"
      ? evmWallets().map((w) => ({ id: w.id, name: w.name, icon: w.icon }))
      : chain === "solana"
        ? solanaWallets().map((w) => ({ id: w.id, name: w.name, icon: w.icon }))
        : [];

  return (
    <div className="wallet-menu" ref={menuRef}>
      <button className="connect-btn" disabled={client.connecting} onClick={toggle} aria-expanded={open}>
        {client.connecting ? "Connecting..." : "Connect Wallet"}
      </button>
      {open ? (
        <div className="wallet-dropdown" role="menu">
          {current ? (
            <>
              <button className="wallet-head wallet-back" onClick={() => { setError(null); setChain(null); }}>
                <span aria-hidden="true">‹</span> {current.name} <span className="wallet-net">{current.net}</span>
              </button>
              {wallets.length ? (
                wallets.map((w) => (
                  <WalletRow
                    key={w.id}
                    name={w.name}
                    icon={w.icon ? <img src={w.icon} alt="" /> : current.icon}
                    onClick={() => go(current.kind, w.id)}
                  />
                ))
              ) : (
                <div className="wallet-empty">No browser wallet found</div>
              )}
              <div className="wallet-divider" />
              <WalletRow
                icon={<WalletConnectIcon />}
                name="WalletConnect"
                sub="Scan with your phone"
                onClick={() => go(current.kind, WALLETCONNECT_ID)}
              />
            </>
          ) : (
            <>
              <div className="wallet-head">Choose a network</div>
              {CHAINS.map((c) => (
                <button key={c.kind} className="wallet-row" onClick={() => pickChain(c.kind)}>
                  <span className="wallet-row-icon">{c.icon}</span>
                  <span className="wallet-row-text">
                    <span className="wallet-row-name">{c.name}</span>
                  </span>
                  <span className="wallet-net">{c.net}</span>
                  <span className="wallet-chevron" aria-hidden="true">›</span>
                </button>
              ))}
            </>
          )}
          {error ? <div className="wallet-error">{error}</div> : null}
        </div>
      ) : null}
    </div>
  );
}
