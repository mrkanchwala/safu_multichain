// Runs part of the engine off the main thread. Two jobs:
//   runs: simulate seeds [from, to) and return each run's summary (same seeds as runMany)
//   fix:  find the highest invested share, in steps, that keeps a quick batch of runs on time
import { runMany, simulate } from "../../../sim/engine/model";
import type { Values } from "../../../sim/engine/settings";
import { LIQ } from "./config";
import type { RunResult } from "./summary";

const { fix_step_pct: FIX_STEP_PCT, fix_quick_runs: FIX_QUICK_RUNS } = LIQ.run;
const WEEK = 7; // chart points: one per week

type Job =
  | { mode: "runs"; s: Values; a: Values; from: number; to: number }
  | { mode: "fix"; s: Values; a: Values };

function runOne(s: Values, a: Values, seed: number): RunResult {
  const m = simulate(s, a, seed);
  const series = m.series
    .filter((_, i) => i % WEEK === 0 || i === m.series.length - 1)
    .map((d) => ({ day: d.day, held: d.assets, owed: d.onClaims, line: d.inLine }));
  return {
    short: m.cashShortDays,
    paidOut: m.paidOut,
    daysToPaid: m.daysToPaid,
    minRatio: m.minRatio,
    series,
    safu: m.safuRevenue / m.years,
    stakerYield: m.stakerYield / m.years,
  };
}

const allOnTime = (s: Values, a: Values, deployPct: number) =>
  runMany({ ...s, deployPct }, { ...a, runs: FIX_QUICK_RUNS }).onTimePct === 100;

/** Binary search over invested share; -1 when even investing nothing fails. */
function quickFix(s: Values, a: Values): number {
  if (!allOnTime(s, a, 0)) return -1;
  let lo = 0;
  let hi = Math.floor((s.deployPct - FIX_STEP_PCT) / FIX_STEP_PCT);
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (allOnTime(s, a, mid * FIX_STEP_PCT)) lo = mid;
    else hi = mid - 1;
  }
  return lo * FIX_STEP_PCT;
}

self.onmessage = ({ data }: MessageEvent<Job>) => {
  if (data.mode === "runs") {
    const out: RunResult[] = [];
    for (let i = data.from; i < data.to; i += 1) out.push(runOne(data.s, data.a, i + 1));
    self.postMessage(out);
  } else {
    self.postMessage(quickFix(data.s, data.a));
  }
};
