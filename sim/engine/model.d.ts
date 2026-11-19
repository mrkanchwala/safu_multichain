// Types for model.js: the parts the page uses.
import type { Values } from "./settings";
export interface DayPoint {
  day: number;
  assets: number;
  owed: number;
  onClaims: number;
  inLine: number;
  stakers: number;
}
export interface RunMetrics {
  cashShortDays: number;
  paidOut: number;
  daysToPaid: number[];
  minRatio: number;
  series: DayPoint[];
  safuRevenue: number;
  stakerYield: number;
  backerYield: number;
  years: number;
  thefts: number;
}
export function simulate(s: Values, a: Values, seed?: number): RunMetrics;
export function runMany(s: Values, a: Values): { onTimePct: number; runs: number };
export const DAYS_PER_YEAR: number;
export const PANIC_DAYS: number;
export const PANIC_CHURN_X: number;
