import { useEffect, useState } from "react";
import { SiteHeader } from "./shell/components/SiteHeader";
import { SiteFooter } from "./shell/components/SiteFooter";
import { AboutSection } from "./shell/components/AboutSection";
import { Whitepaper } from "./shell/components/Whitepaper";
import { Protocols } from "./shell/components/Protocols";
import { Faq } from "./shell/components/Faq";
import { PoolFactsContext } from "./shell/lib/facts";
import { SITE } from "./shell/lib/site";
import type { PoolFaq } from "./shell/components/Faq";
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

function ExplainerPage({ view, onBack, faq }: { view: View; onBack: () => void; faq: PoolFaq }) {
  if (view === "whitepaper") return <Whitepaper onBack={onBack} />;
  if (view === "protocols") return <Protocols onBack={onBack} />;
  return <Faq onBack={onBack} answers={faq} />;
}

/** The pool switcher (B42): one button per enabled chain, hidden when only one is enabled. Switching loads
 *  the other pool's page (`?chain=`), so its own wallet code loads; the hash (#faq …) is kept. */
function ChainSwitcher({ current }: { current: string }) {
  if (SITE.chains.enabled.length < 2) return null;
  const href = (id: string) => `?chain=${id}${window.location.hash}`;
  return (
    <div className="chain-switch" role="group" aria-label="Pool">
      {SITE.chains.enabled.map((id) => (
        <a key={id} href={href(id)} className={`chain-opt${id === current ? " active" : ""}`} aria-current={id === current}>
          {SITE.chains.labels[id]}
        </a>
      ))}
    </div>
  );
}

function Dapp({ adapter, tab, setTab }: { adapter: ChainAdapter; tab: PoolTab; setTab: (t: PoolTab) => void }) {
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
  // Held here, not in Dapp, so the chosen tab survives a visit to the FAQ or whitepaper.
  const [tab, setTab] = useState<PoolTab>("stake");
  const facts = adapter.useFacts();
  const { Badge, Wallet, Hero, Numbers, ExplainerNote } = adapter;

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
        {SITE.chains.enabled.length > 1 ? (
          <div className="wrap switch-bar">
            <ChainSwitcher current={adapter.id} />
            {ExplainerNote && view !== "faq" ? <ExplainerNote /> : null}
          </div>
        ) : null}
        <ExplainerPage view={view} onBack={backToApp} faq={adapter.faq} />
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
          <ChainSwitcher current={adapter.id} />
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

      <Dapp adapter={adapter} tab={tab} setTab={setTab} />

      <Numbers />

      <AboutSection />

      <SiteFooter />
    </PoolFactsContext.Provider>
  );
}
