import { useEffect, useState } from "react";
import { SiteHeader } from "./components/SiteHeader";
import { SiteFooter } from "./components/SiteFooter";
import { AboutSection } from "./components/AboutSection";
import { Whitepaper } from "./components/Whitepaper";
import { Protocols } from "./components/Protocols";
import { StakePanel } from "./components/StakePanel";
import { ClaimsPanel } from "./components/ClaimsPanel";
import { ClaimFilePanel } from "./components/ClaimFilePanel";
import { BackPanel } from "./components/BackPanel";
import { Faq } from "./components/Faq";
import { useClient } from "./lib/client";
import type { AppClient } from "./lib/client";
import { fmtUsdc, readPoolStats } from "./lib/reads";
import { MAX_STAKE_USDC, MIN_STAKE_USDC, POOL_CAP_USDC } from "./lib/network";
import { isLive } from "./lib/soroban";
import { viewFromHash } from "./lib/links";
import type { View } from "./lib/links";

// Borrow/Lend/Backstop tabs removed 2026-09-18 -- there is no lending market
// in this build. Stake (with the yield strip folded in, per Addendum 5's cut
// lever) built 2026-09-18 (A8).
// Four main tabs, 2026-09-24 (founder): Claims' two sub-views became main tabs, backing got its own.
type Tab = "stake" | "back" | "file" | "collect";

// TRY offramp (SEP-6 anchor) removed 2026-09-22, founder decision: not part
// of the v1 build. Payouts stay USDC.
const TABS: { id: Tab; label: string }[] = [
  { id: "stake", label: "Stake" },
  { id: "back", label: "Back the pool" },
  { id: "file", label: "File a claim" },
  { id: "collect", label: "Collect payout" },
];

export default function App() {
  const client = useClient<AppClient>();
  const [tab, setTab] = useState<Tab>("stake");
  const [view, setView] = useState<View>(() => viewFromHash(window.location.hash));
  const [stats, setStats] = useState({ totalStaked: 0n, totalStakers: 0 });

  useEffect(() => {
    let cancelled = false;
    readPoolStats(client).then((s) => {
      if (!cancelled) setStats(s);
    });
    return () => {
      cancelled = true;
    };
  }, [client]);

  // The URL hash picks the page (#faq, #whitepaper), so both explainer
  // pages can be linked directly and browser back/forward works.
  useEffect(() => {
    const onHash = () => setView(viewFromHash(window.location.hash));
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // New page: start at the top, except "#about", which lands on its section.
  useEffect(() => {
    if (view === "main" && window.location.hash === "#about") {
      document.getElementById("about")?.scrollIntoView();
    } else {
      window.scrollTo(0, 0);
    }
  }, [view]);

  const backToApp = () => {
    history.pushState(null, "", window.location.pathname + window.location.search);
    setView("main");
  };

  const badge = !isLive() ? <div className="devnet-badge">Testnet shell, contract not deployed yet</div> : null;

  if (view !== "main") {
    return (
      <>
        {badge}
        <SiteHeader client={client} view={view} />
        {view === "whitepaper" ? (
          <Whitepaper onBack={backToApp} />
        ) : view === "protocols" ? (
          <Protocols onBack={backToApp} />
        ) : (
          <Faq onBack={backToApp} />
        )}
      </>
    );
  }

  return (
    <>
      {badge}
      <SiteHeader client={client} view={view} />

      <section className="hero">
        <div className="wrap">
          <h1>
            SAFU pays your wallet back.
            <br />
            <em>Automatically. No vote. No appeal.</em>
          </h1>
          <p>
            A shared USDC pool on Stellar. Register a wallet on Ethereum, Solana or
            Stellar, and if it's drained through phishing, an approval exploit or a stolen
            key, or a loan in it is liquidated at a price that wasn't real, the payout lands
            automatically, in USDC.
          </p>
          <div className="hero-links">
            <a href="#faq">
              Read the FAQ <span>→</span>
            </a>
            <a href="#whitepaper">
              Read the whitepaper <span>→</span>
            </a>
          </div>
        </div>
      </section>

      <section className="dapp">
        <div className="wrap">
          <div className="dapp-shell">
            <div className="tabs">
              {TABS.map((t) => (
                <button
                  key={t.id}
                  className={`tab${tab === t.id ? " active" : ""}`}
                  onClick={() => setTab(t.id)}
                >
                  {t.label}
                </button>
              ))}
            </div>
            {tab === "stake" ? <StakePanel /> : null}
            {tab === "back" ? <BackPanel /> : null}
            {tab === "file" ? (
              <div className="panel">
                <ClaimFilePanel />
                <div />
              </div>
            ) : null}
            {tab === "collect" ? <ClaimsPanel /> : null}
          </div>
        </div>
      </section>

      <section className="numbers">
        <div className="wrap">
          <h2>The pool</h2>
          <div className="sub">
            Pool cap ${POOL_CAP_USDC.toLocaleString()}. Live numbers, read straight from the pool.
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
              <div className="v">${MIN_STAKE_USDC}</div>
              <div className="k">Min stake</div>
            </div>
            <div className="stat-card">
              <div className="v">${MAX_STAKE_USDC.toLocaleString()}</div>
              <div className="k">Max stake</div>
            </div>
          </div>
        </div>
      </section>

      <AboutSection />

      <SiteFooter />
    </>
  );
}
