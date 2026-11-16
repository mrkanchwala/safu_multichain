import { useEffect, useState } from "react";
import type { ChainAdapter } from "../types";
import { BackPanel } from "./components/BackPanel";
import { ClaimFilePanel } from "./components/ClaimFilePanel";
import { ClaimsPanel } from "./components/ClaimsPanel";
import { StakePanel } from "./components/StakePanel";
import { WalletButton } from "./components/WalletButton";
import { ClientProvider, useClient } from "./lib/client";
import type { AppClient } from "./lib/client";
import { useStakeBounds } from "./lib/useStakeBounds";
import { fmtUsdc, readPoolStats } from "./lib/reads";
import { isLive } from "./lib/soroban";

function Wallet() {
  return <WalletButton client={useClient<AppClient>()} />;
}

function Badge() {
  return isLive() ? null : <div className="devnet-badge">Testnet shell, contract not deployed yet</div>;
}

function Hero() {
  return (
    <p>
      A shared USDC pool on Stellar. Register a wallet on Ethereum, Solana or
      Stellar, and if crypto is stolen from it through phishing, an approval exploit or a
      stolen key, or a loan in it is liquidated at a price that wasn't real, the payout lands
      automatically, in USDC.
    </p>
  );
}

function FilePanel() {
  return (
    <div className="panel">
      <ClaimFilePanel />
      <div />
    </div>
  );
}

function Numbers() {
  const client = useClient<AppClient>();
  const [stats, setStats] = useState({ totalStaked: 0n, totalStakers: 0 });
  const bounds = useStakeBounds();

  useEffect(() => {
    let cancelled = false;
    readPoolStats(client).then((s) => {
      if (!cancelled) setStats(s);
    });
    return () => {
      cancelled = true;
    };
  }, [client]);

  return (
    <section className="numbers">
      <div className="wrap">
        <h2>The pool</h2>
        <div className="sub">
          Pool cap {bounds ? `$${bounds.cap.toLocaleString()}` : "--"}. Live numbers, read straight from the pool.
        </div>
        <div className="stat-grid">
          <div className="stat-card">
            <div className="v">{fmtUsdc(stats.totalStaked)}</div>
            <div className="k">USDC staked</div>
          </div>
          <div className="stat-card">
            <div className="v">{stats.totalStakers}</div>
            <div className="k">Stakers</div>
          </div>
          <div className="stat-card">
            <div className="v">{bounds ? `$${bounds.min}` : "--"}</div>
            <div className="k">Min stake</div>
          </div>
          <div className="stat-card">
            <div className="v">{bounds ? `$${bounds.max.toLocaleString()}` : "--"}</div>
            <div className="k">Max stake</div>
          </div>
        </div>
      </div>
    </section>
  );
}

/** The Stellar pool (USDC on Stellar; Ethereum and Solana wallets stake in over CCTP). */
export const stellar: ChainAdapter = {
  id: "stellar",
  Provider: ClientProvider,
  Wallet,
  Badge,
  Hero,
  panels: { stake: StakePanel, back: BackPanel, file: FilePanel, collect: ClaimsPanel },
  Numbers,
};
