// Turns per-run results from the workers into the page's numbers. Pure, so it is unit-tested.

export interface ChartPoint {
  day: number;
  held: number; // money in the pool
  owed: number; // owed on open claims
  line: number; // claims waiting in line
}

export interface RunResult {
  short: number; // days a payout or exit waited for cash
  paidOut: number;
  daysToPaid: number[];
  minRatio: number;
  series: ChartPoint[];
  safu: number; // per year
  stakerYield: number; // per year
}

export interface Summary {
  runs: number;
  onTimePct: number;
  failedRuns: number;
  paidOutPct: number;
  daysToPaid: number;
  safuPerYear: number;
  stakerYieldPerYear: number;
  typical: ChartPoint[];
  worst: ChartPoint[];
}

export function median(values: number[]): number {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((p, q) => p - q);
  return sorted[Math.floor(sorted.length / 2)];
}

export function summarise(poolCap: number, runs: RunResult[]): Summary {
  const failedRuns = runs.filter((r) => r.short > 0).length;
  const byRatio = [...runs].sort((x, y) => x.minRatio - y.minRatio);
  return {
    runs: runs.length,
    onTimePct: ((runs.length - failedRuns) / runs.length) * 100,
    failedRuns,
    paidOutPct: (median(runs.map((r) => r.paidOut)) / poolCap) * 100,
    daysToPaid: median(runs.flatMap((r) => r.daysToPaid)),
    safuPerYear: median(runs.map((r) => r.safu)),
    stakerYieldPerYear: median(runs.map((r) => r.stakerYield)),
    typical: byRatio[Math.floor(byRatio.length / 2)].series,
    worst: byRatio[0].series,
  };
}

/** Rule of three: with 0 failures in n runs, the true failure rate is likely (95%) under 3 / n. */
export function failureBoundPct(runs: number): number {
  return (3 / runs) * 100;
}
