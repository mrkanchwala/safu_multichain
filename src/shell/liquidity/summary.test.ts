import { describe, expect, it } from "vitest";
import { failureBoundPct, median, summarise } from "./summary";
import type { RunResult } from "./summary";

const run = (over: Partial<RunResult>): RunResult => ({
  short: 0,
  paidOut: 1000,
  daysToPaid: [50],
  minRatio: 1,
  series: [{ day: 0, held: 1, owed: 0, line: 0 }],
  safu: 0,
  stakerYield: 10,
  ...over,
});

describe("summarise", () => {
  it("counts a run with any cash-short day as late", () => {
    const s = summarise(100000, [run({}), run({ short: 3 }), run({}), run({})]);
    expect(s.failedRuns).toBe(1);
    expect(s.onTimePct).toBe(75);
  });

  it("shows the run with the lowest held-to-owed ratio as the worst", () => {
    const worst = [{ day: 0, held: 9, owed: 9, line: 2 }];
    const s = summarise(100000, [run({ minRatio: 2 }), run({ minRatio: 0.4, series: worst }), run({ minRatio: 1.5 })]);
    expect(s.worst).toBe(worst);
  });

  it("reports paid out as a share of the pool cap", () => {
    expect(summarise(100000, [run({ paidOut: 40000 })]).paidOutPct).toBe(40);
  });

  it("gives NaN days to paid when nobody was robbed", () => {
    expect(summarise(100000, [run({ daysToPaid: [] })]).daysToPaid).toBeNaN();
  });
});

it("median and rule of three", () => {
  expect(median([3, 1, 2])).toBe(2);
  expect(failureBoundPct(200)).toBe(1.5);
});
