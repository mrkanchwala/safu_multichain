// The page's state and the pure helpers around it: engine inputs, the sentence's words,
// and the contract's order rules for paired settings. No DOM here, so all of it is unit-tested.
import { ASSUMPTIONS, assumptionDefaults, checkAssumptions } from "../../../sim/engine/assumptions";
import { PANIC_CHURN_X, PANIC_DAYS } from "../../../sim/engine/model";
import { SETTINGS, check, defaults } from "../../../sim/engine/settings";
import type { SettingDef, Values } from "../../../sim/engine/settings";
import type { LiquidityConfig, WhatIf } from "./config";

export interface PageState {
  s: Values; // pool settings
  a: Values; // assumptions about the world
  year: string; // a preset key, or "custom"
  on: Record<string, boolean>; // what-if switches
}

export const CUSTOM = "custom";

// Pairs the contract keeps in order (settings.rs consistent_with_live): low <= high.
const ORDER: [string, string][] = [
  ["minStakePct", "maxStakePct"],
  ["tierB", "tierA"],
  ["tierC", "tierB"],
  ["payoutMidPct", "payoutLowPct"],
  ["payoutHighPct", "payoutMidPct"],
  ["admitMidPct", "admitLowPct"],
  ["admitHighPct", "admitMidPct"],
];

export function initialState(cfg: LiquidityConfig): PageState {
  const first = cfg.years[0];
  return {
    s: defaults(),
    a: {
      ...assumptionDefaults(),
      runs: cfg.run.runs,
      tierAPct: cfg.run.tier_a_pct,
      tierBPct: cfg.run.tier_b_pct,
      theftPctYear: first.rate,
    },
    year: first.key,
    on: {},
  };
}

/** Engine inputs: the state plus whichever what-if switches are on. Off = the engine's default (event off). */
export function engineInputs(st: PageState, switches: WhatIf[]): { s: Values; a: Values } {
  const a = { ...st.a };
  const base = assumptionDefaults();
  for (const w of switches) {
    for (const [k, v] of Object.entries(w.on)) a[k] = st.on[w.key] ? v : base[k];
  }
  return { s: { ...st.s }, a };
}

/** Moving one slider past its partner pulls the partner along, outward, so chains like A >= B >= C hold. */
export function keepOrder(values: Values, moved: string): Values {
  const out = { ...values };
  const queue = [moved];
  while (queue.length) {
    const k = queue.shift() as string;
    for (const [lo, hi] of ORDER) {
      if (!(lo in out) || out[lo] <= out[hi]) continue;
      if (k === hi) {
        out[lo] = out[hi];
        queue.push(lo);
      } else if (k === lo) {
        out[hi] = out[lo];
        queue.push(hi);
      }
    }
  }
  return out;
}

const round = (v: number, places: number) => Math.round(v * 10 ** places) / 10 ** places;

export function rateWord(st: PageState): string {
  return `${round(st.a.theftPctYear, 2)}%`;
}

export function whatWord(st: PageState, switches: WhatIf[]): string {
  const on = switches.filter((w) => st.on[w.key]).map((w) => w.phrase);
  if (on.length === 0) return "nothing goes wrong";
  if (on.length > 2 && on.length === switches.length) return "everything goes wrong at once";
  if (on.length === 1) return on[0];
  return `${on.slice(0, -1).join(", ")} and ${on[on.length - 1]}`;
}

/** The yield split as a phrase. Only the stakers' share is shown (founder 19 Nov: the backer share confused people). */
export function splitWord(s: Values): string {
  return s.stakerYieldPct === 100 ? "all yield to stakers" : `${s.stakerYieldPct}% of yield to stakers`;
}

export interface StressBand {
  from: number;
  to: number;
}

/** Shaded days on the chart for each switch that is on: the event's first day through its last. */
export function stressBands(st: PageState, switches: WhatIf[]): StressBand[] {
  return switches
    .filter((w) => st.on[w.key])
    .map((w) => {
      const [startKey, from] = Object.entries(w.on).find(([k]) => k.endsWith("Day")) ?? ["", 0];
      const length = w.on.freezeDays ?? (startKey === "panicDay" ? PANIC_DAYS : 1);
      return { from, to: from + length };
    });
}

/** "{name}" in switch copy = that value from the switch, or the engine's panic constants. */
export function fillCopy(text: string, w: WhatIf): string {
  const values: Values = { ...w.on, panicDays: PANIC_DAYS, panicX: PANIC_CHURN_X };
  return text.replace(/\{(\w+)\}/g, (m, k: string) => (k in values ? String(values[k]) : m));
}

/** The sliders a visitor sees: payout-relevant settings, and assumptions that aren't driven by the sentence. */
export function shownSettings(cfg: LiquidityConfig): SettingDef[] {
  return SETTINGS.filter((d) => d.liquidity && !cfg.run.hidden_settings.includes(d.key));
}

export function shownAssumptions(cfg: LiquidityConfig): SettingDef[] {
  return ASSUMPTIONS.filter((d) => !cfg.run.hidden_assumptions.includes(d.key));
}

export function settingDef(key: string): SettingDef {
  const d = SETTINGS.find((x) => x.key === key) ?? ASSUMPTIONS.find((x) => x.key === key);
  if (!d) throw new Error(`unknown setting ${key}`);
  return d;
}

/** Whole dollars, never below zero (a run's yield can dip negative while invested money is frozen). */
export function formatUsd(v: number): string {
  return `$${Math.round(Math.max(v, 0)).toLocaleString("en-US")}`;
}

export function formatValue(v: number, unit: string): string {
  if (unit === "$") return formatUsd(v);
  if (unit === "x" || unit === "%") return `${v}${unit}`;
  return `${v} ${unit}`;
}

/** Share links: the page state rides in the hash after "#liquidity/". Bad links fall back to defaults. */
export function encodeState(st: PageState): string {
  return encodeURIComponent(JSON.stringify(st));
}

/** Hidden settings always stay at their contract default, so a shared link can't set them. */
function hiddenDefaults(cfg: LiquidityConfig): Values {
  const d = defaults();
  return Object.fromEntries(cfg.run.hidden_settings.map((k) => [k, d[k]]));
}

/** Known keys only, finite numbers only, each clamped to its definition's range. */
function cleanValues(raw: unknown, defs: SettingDef[], base: Values): Values {
  const out = { ...base };
  if (typeof raw !== "object" || raw === null) return out;
  const given = raw as Record<string, unknown>;
  for (const d of defs) {
    const v = given[d.key];
    if (typeof v === "number" && Number.isFinite(v)) out[d.key] = Math.min(Math.max(v, d.min), d.max);
  }
  return out;
}

/** A shared link is untrusted input (CSO 19 Nov): anything out of range would hang the visitor's tab. */
export function decodeState(text: string, cfg: LiquidityConfig): PageState {
  const base = initialState(cfg);
  try {
    const raw = JSON.parse(decodeURIComponent(text)) as Record<string, unknown>;
    const years = new Set([...cfg.years.map((y) => y.key), CUSTOM]);
    const on = (raw.on ?? {}) as Record<string, unknown>;
    const a = cleanValues(raw.a, ASSUMPTIONS, base.a);
    a.runs = cfg.run.runs;
    a.theftPctYear = Math.min(a.theftPctYear, cfg.run.custom_max_pct);
    return {
      s: { ...cleanValues(raw.s, SETTINGS, base.s), ...hiddenDefaults(cfg) },
      a,
      year: typeof raw.year === "string" && years.has(raw.year) ? raw.year : base.year,
      on: Object.fromEntries(cfg.switches.map((w) => [w.key, on[w.key] === true])),
    };
  } catch {
    return base;
  }
}

/** Why the model can't run on these inputs: the contract's setting rules plus the engine's assumption rules. */
export function inputProblems(st: PageState): string[] {
  return [...check(st.s), ...checkAssumptions(st.a)];
}
