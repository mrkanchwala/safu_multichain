import { useEffect, useState } from "react";
import { SiteHeader } from "./shell/components/SiteHeader";
import { SiteFooter } from "./shell/components/SiteFooter";
import { AboutSection } from "./shell/components/AboutSection";
import { Whitepaper } from "./shell/components/Whitepaper";
import { Protocols } from "./shell/components/Protocols";
import { Faq } from "./shell/components/Faq";
import { PoolFactsContext } from "./shell/lib/facts";
import { scrollToFaqAnswer, viewFromHash } from "./shell/lib/links";
import type { View } from "./shell/lib/links";
import type { ChainAdapter, PoolTab } from "./chains/types";

// The four dapp tabs every pool has (2026-09-24); each adapter supplies the panel behind a tab.
const TABS: { id: PoolTab; label: string }[] = [
  { id: "stake", label: "Stake" },
  { id: "back", label: "Back the pool" },
  { id: "file", label: "File a claim" },
  { id: "collect", label: "Collect payout" },
];

/** URL hash → page (#faq, #whitepaper, #protocols), so explainer pages can be linked and back works. */
function useHashView(): [View, (v: View) => void] {
  const [view, setView] = useState<View>(() => viewFromHash(window.location.hash));

  useEffect(() => {
    const onHash = () => {
      setView(viewFromHash(window.location.hash));
      scrollToFaqAnswer(window.location.hash); // already on the FAQ: the view doesn't change
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // New page: start at the top, except "#about", which lands on its section.
  useEffect(() => {
    if (view === "main" && window.location.hash === "#about") {
      document.getElementById("about")?.scrollIntoView();
    } else if (view === "faq" && scrollToFaqAnswer(window.location.hash)) {
      // "#faq/<id>": landed on that answer
    } else {
      window.scrollTo(0, 0);
    }
  }, [view]);

  return [view, setView];
}

function ExplainerPage({ view, onBack }: { view: View; onBack: () => void }) {
  if (view === "whitepaper") return <Whitepaper onBack={onBack} />;
  if (view === "protocols") return <Protocols onBack={onBack} />;
  return <Faq onBack={onBack} />;
}

function Dapp({ adapter }: { adapter: ChainAdapter }) {
  const [tab, setTab] = useState<PoolTab>("stake");
  const Panel = adapter.panels[tab];
  return (
    <section className="dapp">
      <div className="wrap">
        <div className="dapp-shell">
          <div className="tabs">
            {TABS.map((t) => (
              <button key={t.id} className={`tab${tab === t.id ? " active" : ""}`} onClick={() => setTab(t.id)}>
                {t.label}
              </button>
            ))}
          </div>
          <Panel />
        </div>
      </div>
    </section>
  );
}

export default function App({ adapter }: { adapter: ChainAdapter }) {
  const [view, setView] = useHashView();
  const facts = adapter.useFacts();
  const { Badge, Wallet, Hero, Numbers } = adapter;

  const backToApp = () => {
    history.pushState(null, "", window.location.pathname + window.location.search);
    setView("main");
  };

  const header = <SiteHeader wallet={<Wallet />} view={view} />;

  if (view !== "main") {
    return (
      <PoolFactsContext.Provider value={facts}>
        <Badge />
        {header}
        <ExplainerPage view={view} onBack={backToApp} />
      </PoolFactsContext.Provider>
    );
  }

  return (
    <PoolFactsContext.Provider value={facts}>
      <Badge />
      {header}

      <section className="hero">
        <div className="wrap">
          <h1>
            SAFU pays you back when your crypto is stolen.
            <br />
            <em>Automatically. No vote. No appeal.</em>
          </h1>
          <Hero />
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

      <Dapp adapter={adapter} />

      <Numbers />

      <AboutSection />

      <SiteFooter />
    </PoolFactsContext.Provider>
  );
}
