import { useState } from "react";
import type { ReactNode } from "react";
import { DAYS_PER_YEAR } from "../../../sim/engine/model";
import { FIXED } from "../../../sim/engine/settings";
import type { Values } from "../../../sim/engine/settings";
import { LIQ } from "./config";
import { formatUsd as usd, shownAssumptions, shownSettings } from "./model-state";
import { Slider } from "./Slider";
import { failureBoundPct } from "./summary";

export type PanelId = "settings" | "math";

function Panel({ title, open, onClose, children }: { title: string; open: boolean; onClose: () => void; children: ReactNode }) {
  return (
    <aside className={`liq-panel${open ? " on" : ""}`} aria-label={title} aria-hidden={!open}>
      <button className="liq-close" aria-label="Close" onClick={onClose}>
        ×
      </button>
      <h2>{title}</h2>
      {children}
    </aside>
  );
}

export function SettingsPanel(props: {
  open: boolean;
  onClose: () => void;
  s: Values;
  a: Values;
  problems: string[];
  onSetting: (key: string, v: number) => void;
  onAssumption: (key: string, v: number) => void;
}) {
  const shown = shownSettings(LIQ);
  const groups = [...new Set(shown.map((d) => d.group))];
  return (
    <Panel title="Settings" open={props.open} onClose={props.onClose}>
      <p className="liq-note">{LIQ.copy.settings_note}</p>
      {groups.map((g) => (
        <div key={g}>
          <h3>{g}</h3>
          {shown
            .filter((d) => d.group === g)
            .map((d) => (
              <Slider key={d.key} def={d} values={props.s} onChange={props.onSetting} />
            ))}
        </div>
      ))}
      <h3>Assumptions about the world</h3>
      {shownAssumptions(LIQ).map((d) => (
        <Slider key={d.key} def={d} values={props.a} onChange={props.onAssumption} />
      ))}
      <p className="liq-err">{props.problems.join(" · ")}</p>
    </Panel>
  );
}

/** The seven steps of the model, every number read from the engine and the current settings. */
function Formulas({ s, a }: { s: Values; a: Values }) {
  return (
    <ol>
      <li>
        <b>Thefts.</b> Each day, each staker without an open claim is robbed with chance
        <span className="liq-eq">
          p = <i>r</i> / {DAYS_PER_YEAR} ( + <i>b</i> on the mass-theft day )
        </span>
        where <i>r</i> is the yearly rate you picked and <i>b</i> the share hit in a mass theft. The amount stolen is
        random, averaging {usd(a.avgLoss)}.
      </li>
      <li>
        <b>What a claim is owed.</b> Never more than the loss, never more than the stake times its tier multiple. A
        stake must be {s.timeGateDays} days old before its claim is approved.
        <span className="liq-eq">
          owed = min( loss , <i>m</i>
          <sub>tier</sub> × stake ) <i>m</i> ∈ {"{"}
          {s.tierA}, {s.tierB}, {s.tierC}
          {"}"}
        </span>
      </li>
      <li>
        <b>How full the pool is.</b>
        <span className="liq-eq">u = owed on open claims / ( stakes + backing )</span>
        Calm below {FIXED.utilLowPct}%, busy below {FIXED.utilMidPct}%, stressed from {FIXED.utilMidPct}%.
      </li>
      <li>
        <b>New claims enter</b> only while they fit, in the order they came in.
        <span className="liq-eq">owed + new claim ≤ stakes + backing</span>
        Per day: at most {s.admitLowPct}% / {s.admitMidPct}% / {s.admitHighPct}% of the pool (calm / busy / stressed),
        and at most one claim per {FIXED.oracleDivisor} stakers.
      </li>
      <li>
        <b>Payouts.</b> After the {s.cooldownDays}-day check, each claim is paid evenly over {s.vestingDays} days. The
        day's total is capped and shared by what each claim is still owed.
        <span className="liq-eq">
          claim's share today = <i>c</i>(u) × pool × its unpaid amount / all unpaid amounts <i>c</i> ={" "}
          {s.payoutLowPct}% / {s.payoutMidPct}% / {s.payoutHighPct}%
        </span>
      </li>
      <li>
        <b>Cash.</b> Up to {s.deployPct}% of the pool earns yield in the lending venue, and the rest stays as cash.
        When cash runs low the pool takes money back out, except while the venue is frozen.
      </li>
      <li>
        <b>The result.</b> A run is on time when no payout or exit ever waits for cash.
        <span className="liq-eq">result = runs on time / {a.runs} × 100%</span>
        Each run is {a.years} years, day by day. When all {a.runs} runs are on time, the true failure rate is still
        likely under {failureBoundPct(a.runs).toFixed(1)}% (the "rule of three": 3 ÷ runs, 95% confidence).
      </li>
    </ol>
  );
}

export function MathPanel({ open, onClose, s, a }: { open: boolean; onClose: () => void; s: Values; a: Values }) {
  const [showFormulas, setShowFormulas] = useState(false);
  return (
    <Panel title="The math" open={open} onClose={onClose}>
      <div className="liq-plain">
        <p>
          Each run plays out {a.years} years of the pool, one day at a time, with thefts landing on random days at the
          rate you picked.
        </p>
        <p>
          Claims are paid in the order they arrive, spread over days under a daily limit, using the same rules as the
          pool's contract code.
        </p>
        <p>A run counts as on time when no payout and no exit ever had to wait for cash.</p>
        <p>{LIQ.copy.revenue_note}</p>
      </div>
      <button className="liq-link" onClick={() => setShowFormulas((v) => !v)}>
        {showFormulas ? "Hide formulas" : "Show formulas"}
      </button>
      {showFormulas ? (
        <div className="liq-math">
          <Formulas s={s} a={a} />
          <p className="liq-src">{LIQ.copy.audit_note}</p>
        </div>
      ) : null}
    </Panel>
  );
}
