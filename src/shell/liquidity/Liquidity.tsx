// Liquidity tab (F20, founder 19 Nov): "How the pool pays". One sentence with tap-to-change words,
// the live result, a revenue line, a chart, and Settings / The math as side panels.
import { useEffect, useRef, useState } from "react";
import { defaults } from "../../../sim/engine/settings";
import { LIQUIDITY_STATE_PREFIX as HASH_PREFIX } from "../lib/links";
import { SITE } from "../lib/site";
import { LIQ } from "./config";
import { LiquidityChart } from "./LiquidityChart";
import { RateMenu, SliderMenu, Tap, WhatMenu } from "./Menus";
import {
  CUSTOM,
  decodeState,
  encodeState,
  engineInputs,
  formatUsd as usd,
  initialState,
  inputProblems,
  keepOrder,
  rateWord,
  splitWord,
  stressBands,
  whatWord,
} from "./model-state";
import type { PageState } from "./model-state";
import { MathPanel, SettingsPanel } from "./Panels";
import type { PanelId } from "./Panels";
import { Runner } from "./runner";
import type { Result } from "./runner";
import { failureBoundPct } from "./summary";
import "./liquidity.css";

const DEBOUNCE_MS = 250;
const HEADLINE_MULTIPLE = defaults().tierA;

type Menu = "rate" | "what" | "yield" | "split";

function startState(): PageState {
  const h = window.location.hash;
  return h.startsWith(HASH_PREFIX) ? decodeState(h.slice(HASH_PREFIX.length), LIQ) : initialState(LIQ);
}

const MODEL_ERROR = "The model hit an error. Change a setting to run it again.";

/** Runs the model whenever the inputs settle; returns the latest result, whether a run is in flight,
 *  and any reason it can't run (bad inputs, or a worker crash). */
function useModel(st: PageState): { result: Result | null; busy: boolean; problems: string[] } {
  const runner = useRef<Runner | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const inputErrors = inputProblems(st);
  const ok = inputErrors.length === 0;
  const problems = failed ? [...inputErrors, MODEL_ERROR] : inputErrors;

  useEffect(() => {
    runner.current = new Runner();
    return () => runner.current?.cancel();
  }, []);

  useEffect(() => {
    if (!ok) {
      runner.current?.cancel(); // no stale result next to an error
      setBusy(false);
      return;
    }
    const timer = setTimeout(() => {
      history.replaceState(null, "", HASH_PREFIX + encodeState(st));
      setBusy(true);
      setFailed(false);
      const { s, a } = engineInputs(st, LIQ.switches);
      const onResult = (r: Result) => {
        setResult(r);
        if (r.summary.failedRuns === 0 || r.safeDeployPct !== undefined) setBusy(false);
      };
      const onError = (e: unknown) => {
        console.error(e);
        setFailed(true);
        setBusy(false);
      };
      void runner.current?.run(s, a, onResult, onError);
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [st, ok]);

  return { result, busy, problems };
}

function Facts({ result }: { result: Result }) {
  const o = result.summary;
  const days = Number.isFinite(o.daysToPaid) ? `~${o.daysToPaid} days` : "no thefts";
  return (
    <div className="liq-facts">
      <b>{o.paidOutPct.toFixed(1)}%</b> of the pool paid out · <b>{days}</b> from theft to paid ·{" "}
      {o.failedRuns === 0 ? (
        <>
          true failure rate likely under <b>{failureBoundPct(o.runs).toFixed(1)}%</b>
        </>
      ) : (
        <>
          <b>{o.failedRuns}</b> runs had a payout or exit wait for cash
        </>
      )}
    </div>
  );
}

function FixLine({ result, deployPct, onApply }: { result: Result; deployPct: number; onApply: (pct: number) => void }) {
  const safe = result.safeDeployPct;
  if (result.summary.failedRuns === 0 || safe === undefined) return null;
  if (safe < 0) {
    return <div className="liq-fix">Investing less is not enough here. Try a lower payout multiple or a longer payout spread in Settings.</div>;
  }
  return (
    <div className="liq-fix">
      Invest {safe}% of the pool instead of {deployPct}% and every claim is paid on time.{" "}
      <button onClick={() => onApply(safe)}>Apply</button>
    </div>
  );
}

export default function Liquidity() {
  const [st, setSt] = useState<PageState>(startState);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [panel, setPanel] = useState<PanelId | null>(null);
  const [run, setRun] = useState<"typical" | "worst">("typical");
  const [shareLabel, setShareLabel] = useState("Share");
  const story = useRef<HTMLDivElement>(null);
  const story2 = useRef<HTMLDivElement>(null);
  const { result, busy, problems } = useModel(st);

  useEffect(() => {
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setMenu(null);
      setPanel(null);
    };
    document.addEventListener("click", close);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", close);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const toggle = (m: Menu) => () => setMenu((cur) => (cur === m ? null : m));
  const setSetting = (key: string, v: number) => setSt((p) => ({ ...p, s: keepOrder({ ...p.s, [key]: v }, key) }));
  const setAssumption = (key: string, v: number) => setSt((p) => ({ ...p, a: { ...p.a, [key]: v } }));
  const pickYear = (key: string, rate?: number) => {
    setSt((p) => ({ ...p, year: key, a: rate === undefined ? p.a : { ...p.a, theftPctYear: rate } }));
    if (key !== CUSTOM) setMenu(null);
  };
  const share = async () => {
    try {
      await navigator.clipboard.writeText(window.location.href);
      setShareLabel("Link copied");
    } catch {
      setShareLabel("Copy the address bar");
    }
  };

  const o = result?.summary;
  return (
    <>
      <main className="liq">
        <div className="wrap">
          <h1>{LIQ.copy.title}</h1>
          <p className="liq-lede">
            {SITE.hero.headline} Crypto stolen? Payouts are automatic, up to {HEADLINE_MULTIPLE}x their stake.
          </p>

          <div className="liq-hero">
            <div className="liq-hero-text">
              <div className="liq-story" ref={story} aria-live="polite">
                If{" "}
                <Tap label={rateWord(st)} open={menu === "rate"} onToggle={toggle("rate")} story={story}>
                  <RateMenu st={st} onPick={pickYear} onCustom={(v) => setAssumption("theftPctYear", v)} />
                </Tap>{" "}
                of stakers are robbed each year and{" "}
                <Tap label={whatWord(st, LIQ.switches)} open={menu === "what"} onToggle={toggle("what")} story={story}>
                  <WhatMenu st={st} onSwitch={(k, on) => setSt((p) => ({ ...p, on: { ...p.on, [k]: on } }))} />
                </Tap>
                , the pool pays every claim on time in
                <div className="liq-answer">
                  <span className={`liq-big${o ? (o.failedRuns === 0 ? " ok" : " bad") : ""}`}>{o ? `${Math.round(o.onTimePct)}%` : "…"}</span>
                  <span className="liq-of">
                    of {st.a.runs} simulated runs.<span className={`liq-busy${busy ? " on" : ""}`}>running…</span>
                  </span>
                </div>
              </div>
              {problems.length ? <div className="liq-fix">This setup can't run: {problems.join(" · ")}. Change it in Settings.</div> : null}
              {result && !problems.length ? <Facts result={result} /> : null}
              {result && !problems.length ? <FixLine result={result} deployPct={st.s.deployPct} onApply={(pct) => setSetting("deployPct", pct)} /> : null}
            </div>

            <div className="liq-chart-wrap">
              <div className="liq-chart-top">
                <div className="liq-chart-title">{LIQ.copy.chart_title}</div>
                <div className="liq-seg">
                  <button aria-pressed={run === "typical"} onClick={() => setRun("typical")}>
                    Typical run
                  </button>
                  <button aria-pressed={run === "worst"} onClick={() => setRun("worst")}>
                    Worst run
                  </button>
                </div>
              </div>
              {o ? <LiquidityChart data={o[run]} bands={stressBands(st, LIQ.switches)} title={LIQ.copy.chart_title} /> : <div className="liq-chart-empty" />}
            </div>
          </div>

          <div className="liq-under">
            <div className="liq-links">
              <button className="liq-link" onClick={() => setPanel("settings")}>
                Settings
              </button>
              <button className="liq-link" onClick={() => setPanel("math")}>
                The math
              </button>
            </div>

            <div className="liq-story liq-story2" ref={story2}>
              At{" "}
              <Tap label={`${Math.round(st.a.apyPct * 10) / 10}%`} open={menu === "yield"} onToggle={toggle("yield")} story={story2}>
                <SliderMenu keys={["apyPct"]} values={st.a} onChange={setAssumption} />
              </Tap>{" "}
              lending yield, with{" "}
              <Tap label={splitWord(st.s)} open={menu === "split"} onToggle={toggle("split")} story={story2}>
                <SliderMenu keys={["stakerYieldPct"]} values={st.s} onChange={setSetting} />
              </Tap>
              , SAFU earns <b>{o ? usd(o.safuPerYear) : "…"}</b> a year.
            </div>
            {o ? (
              <div className="liq-facts">
                Stakers earn <b>{usd(o.stakerYieldPerYear)}</b> a year in yield on a <b>{usd(st.s.poolCap)}</b> pool
                (median of all runs).
              </div>
            ) : null}

            <div className="liq-cta">
              <button className="liq-btn" onClick={share}>
                {shareLabel}
              </button>
              <a className="liq-btn primary" href="#">
                Stake in the pool
              </a>
            </div>
          </div>
        </div>
      </main>

      <div className={`liq-scrim${panel ? " on" : ""}`} onClick={() => setPanel(null)} />
      <SettingsPanel
        open={panel === "settings"}
        onClose={() => setPanel(null)}
        s={st.s}
        a={st.a}
        problems={problems}
        onSetting={setSetting}
        onAssumption={setAssumption}
      />
      <MathPanel open={panel === "math"} onClose={() => setPanel(null)} s={st.s} a={st.a} />
    </>
  );
}
