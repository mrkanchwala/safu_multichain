import type { ChainAdapter } from "../types";
import type { PoolFacts } from "../../shell/lib/facts";
import { BackPanel } from "./components/BackPanel";
import { ClaimFilePanel } from "./components/ClaimFilePanel";
import { ClaimsPanel } from "./components/ClaimsPanel";
import { StakePanel } from "./components/StakePanel";
import { WalletButton } from "./components/WalletButton";
import { SOLANA_FAQ } from "./faq";
import { ClientProvider, useClient } from "./lib/client";
import { POOL } from "./lib/pool";
import { fmtSol, stakeBounds } from "./lib/reads";
import { usePool } from "./lib/usePool";

// The Solana pool (B42, founder 16 Nov): the Colosseum devnet SOL pool that solana.safustaking.com served,
// ported from safu_solana e32e0af + the 31 Oct wording patches. Not the USDC pool (hidden until the grant).

const NET = POOL.cluster === "devnet" ? "Solana devnet" : "a local Solana test chain";

function Wallet() {
  return <WalletButton client={useClient()} />;
}

/** Shown only when the pool can't be read (founder, 2026-10-01: no standing devnet/demo banner). */
function Badge() {
  const { error } = usePool();
  return error ? <div className="devnet-badge">The pool isn't reachable on {NET} right now</div> : null;
}

function Hero() {
  return (
    <p>
      A shared SOL pool on Solana. Stake, register the wallets you want covered, and if crypto is stolen from one
      through phishing, a bad approval or a stolen key, the payout comes back to you in SOL. While it waits, the
      pool's SOL earns staking yield through Marinade.
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
  const { pool } = usePool();
  const stats = [
    { v: pool ? fmtSol(pool.total_staked, 2) : "--", k: "SOL staked" },
    { v: pool ? pool.total_stakers.toString() : "--", k: "Stakers" },
    { v: pool ? fmtSol(pool.total_backed, 2) : "--", k: "SOL backing" },
    { v: pool ? fmtSol(stakeBounds(pool).max, 2) : "--", k: "Max stake (SOL)" },
  ];
  return (
    <section className="numbers">
      <div className="wrap">
        <h2>The pool</h2>
        <div className="sub">Live numbers, read straight from the pool on {NET}.</div>
        <div className="stat-grid">
          {stats.map((s) => (
            <div className="stat-card" key={s.k}>
              <div className="v">{s.v}</div>
              <div className="k">{s.k}</div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function ExplainerNote() {
  return (
    <p className="panel-note">
      These pages describe SAFU's Stellar pool. For how the Solana test pool differs, see the <a href="#faq">FAQ</a>.
    </p>
  );
}

const noFacts = (): PoolFacts => ({});

/** The Solana test pool (SOL on Solana devnet, Marinade yield). */
export const solana: ChainAdapter = {
  id: "solana",
  Provider: ClientProvider,
  Wallet,
  Badge,
  Hero,
  panels: { stake: StakePanel, back: BackPanel, file: FilePanel, collect: ClaimsPanel },
  Numbers,
  useFacts: noFacts,
  faq: SOLANA_FAQ,
  ExplainerNote,
};
