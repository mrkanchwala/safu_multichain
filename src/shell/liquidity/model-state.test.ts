import { describe, expect, it } from "vitest";
import { ASSUMPTIONS, assumptionDefaults } from "../../../sim/engine/assumptions";
import { PANIC_DAYS } from "../../../sim/engine/model";
import { SETTINGS, check, defaults } from "../../../sim/engine/settings";
import type { Values } from "../../../sim/engine/settings";
import { viewFromHash } from "../lib/links";
import { LIQ } from "./config";
import {
  CUSTOM,
  decodeState,
  encodeState,
  engineInputs,
  fillCopy,
  initialState,
  inputProblems,
  keepOrder,
  shownSettings,
  splitWord,
  stressBands,
  whatWord,
} from "./model-state";

const engineKeys = new Set([...SETTINGS, ...ASSUMPTIONS].map((d) => d.key));

describe("hq/liquidity.toml agrees with the engine", () => {
  it("names only real settings and assumptions", () => {
    const named = [...LIQ.run.hidden_settings, ...LIQ.run.hidden_assumptions, ...LIQ.switches.flatMap((w) => Object.keys(w.on))];
    expect(named.filter((k) => !engineKeys.has(k))).toEqual([]);
  });

  it("keeps every year preset within the custom cap and the engine's bounds", () => {
    const theft = ASSUMPTIONS.find((d) => d.key === "theftPctYear");
    for (const y of LIQ.years) {
      expect(y.rate).toBeLessThanOrEqual(LIQ.run.custom_max_pct);
      expect(y.rate).toBeGreaterThanOrEqual(theft?.min ?? 0);
    }
    expect(LIQ.run.custom_max_pct).toBeLessThanOrEqual(theft?.max ?? 0);
  });

  it("starts from valid pool settings", () => {
    expect(check(initialState(LIQ).s)).toEqual([]);
  });

  it("leaves no {placeholder} unfilled in switch copy", () => {
    for (const w of LIQ.switches) expect(fillCopy(`${w.label} ${w.tip}`, w)).not.toMatch(/\{\w+\}/);
  });
});

describe("keepOrder", () => {
  it("pulls the whole tier chain down when tier A drops to 1", () => {
    const out = keepOrder({ ...defaults(), tierA: 1 }, "tierA");
    expect([out.tierA, out.tierB, out.tierC]).toEqual([1, 1, 1]);
    expect(check(out)).toEqual([]);
  });

  it("lets tier B rise and pushes tier A up with it", () => {
    const out = keepOrder({ ...defaults(), tierB: 18 }, "tierB");
    expect(out.tierA).toBe(18);
    expect(out.tierC).toBe(defaults().tierC);
  });

  it("keeps smallest stake at or under largest", () => {
    const out = keepOrder({ ...defaults(), minStakePct: 1 }, "minStakePct");
    expect(out.maxStakePct).toBe(1);
  });

  it("never mutates its input", () => {
    const input: Values = { ...defaults(), tierA: 1 };
    keepOrder(input, "tierA");
    expect(input.tierB).toBe(defaults().tierB);
  });
});

describe("engine inputs", () => {
  it("switch off = the engine default, switch on = the switch's values", () => {
    const st = initialState(LIQ);
    const off = engineInputs(st, LIQ.switches).a;
    const base = assumptionDefaults();
    for (const w of LIQ.switches) for (const k of Object.keys(w.on)) expect(off[k]).toBe(base[k]);
    const allOn = engineInputs({ ...st, on: Object.fromEntries(LIQ.switches.map((w) => [w.key, true])) }, LIQ.switches).a;
    for (const w of LIQ.switches) for (const [k, v] of Object.entries(w.on)) expect(allOn[k]).toBe(v);
  });

  it("uses the page's run count and the first year's rate", () => {
    const { a } = engineInputs(initialState(LIQ), LIQ.switches);
    expect(a.runs).toBe(LIQ.run.runs);
    expect(a.theftPctYear).toBe(LIQ.years[0].rate);
  });
});

describe("sentence words", () => {
  const st = initialState(LIQ);
  it("says everything only when every switch is on", () => {
    const all = { ...st, on: Object.fromEntries(LIQ.switches.map((w) => [w.key, true])) };
    expect(whatWord(all, LIQ.switches)).toBe("everything goes wrong at once");
    const [a, b] = LIQ.switches;
    expect(whatWord({ ...st, on: { [a.key]: true, [b.key]: true } }, LIQ.switches)).toBe(`${a.phrase} and ${b.phrase}`);
  });

  it("names the switches that are on", () => {
    expect(whatWord(st, LIQ.switches)).toBe("nothing goes wrong");
    const one = { ...st, on: { [LIQ.switches[0].key]: true } };
    expect(whatWord(one, LIQ.switches)).toBe(LIQ.switches[0].phrase);
  });

  it("describes the yield split", () => {
    expect(splitWord({ stakerYieldPct: 100 })).toBe("all yield to stakers");
    expect(splitWord({ stakerYieldPct: 60 })).toBe("60% of yield to stakers");
  });

  it("shades each switched-on event on the chart", () => {
    const panic = LIQ.switches.find((w) => w.key === "panic");
    if (!panic) throw new Error("panic switch missing");
    const bands = stressBands({ ...st, on: { panic: true } }, LIQ.switches);
    expect(bands).toEqual([{ from: panic.on.panicDay, to: panic.on.panicDay + PANIC_DAYS }]);
  });
});

describe("share links", () => {
  it("round-trip the page state", () => {
    const st = { ...initialState(LIQ), year: CUSTOM, on: { freeze: true } };
    st.a = { ...st.a, theftPctYear: 3 };
    const back = decodeState(encodeState(st), LIQ);
    expect(back.a.theftPctYear).toBe(3);
    expect(back.on).toEqual(Object.fromEntries(LIQ.switches.map((w) => [w.key, w.key === "freeze"])));
  });

  it("never let a link set a hidden setting", () => {
    const st = initialState(LIQ);
    st.s = { ...st.s, backerYieldPct: 10 };
    expect(decodeState(encodeState(st), LIQ).s.backerYieldPct).toBe(defaults().backerYieldPct);
  });

  it("clamp every value a link carries to its range, and drop the rest", () => {
    const evil = encodeURIComponent(
      JSON.stringify({ s: { tierA: 1e9, poolCap: "1e12", bogus: 5 }, a: { years: 1e6, startFillPct: 1e9, avgLoss: -5 }, year: "x", on: { freeze: "yes", panic: true, zzz: true } }),
    );
    const st = decodeState(evil, LIQ);
    const max = (k: string) => [...SETTINGS, ...ASSUMPTIONS].find((d) => d.key === k)?.max;
    const min = (k: string) => [...SETTINGS, ...ASSUMPTIONS].find((d) => d.key === k)?.min;
    expect(st.s.tierA).toBe(max("tierA"));
    expect(st.s.poolCap).toBe(defaults().poolCap);
    expect("bogus" in st.s).toBe(false);
    expect(st.a.years).toBe(max("years"));
    expect(st.a.startFillPct).toBe(max("startFillPct"));
    expect(st.a.avgLoss).toBe(min("avgLoss"));
    expect(st.year).toBe(LIQ.years[0].key);
    expect(st.on).toEqual(Object.fromEntries(LIQ.switches.map((w) => [w.key, w.key === "panic"])));
  });

  it("block the run when the wallet mix is over 100%", () => {
    const st = initialState(LIQ);
    expect(inputProblems(st)).toEqual([]);
    expect(inputProblems({ ...st, a: { ...st.a, tierAPct: 60, tierBPct: 60 } }).length).toBe(1);
  });

  it("clamp the theft rate to the custom cap and ignore a bad link", () => {
    const st = initialState(LIQ);
    st.a = { ...st.a, theftPctYear: 50 };
    expect(decodeState(encodeState(st), LIQ).a.theftPctYear).toBe(LIQ.run.custom_max_pct);
    expect(decodeState("%%not-json", LIQ)).toEqual(initialState(LIQ));
  });

  it("route to the Liquidity tab", () => {
    expect(viewFromHash("#liquidity")).toBe("liquidity");
    expect(viewFromHash(`#liquidity/${encodeState(initialState(LIQ))}`)).toBe("liquidity");
    expect(viewFromHash("#liquidityx")).toBe("main");
  });
});

it("hides the settings the founder ruled out", () => {
  const shown = shownSettings(LIQ).map((d) => d.key);
  for (const k of LIQ.run.hidden_settings) expect(shown).not.toContain(k);
});
