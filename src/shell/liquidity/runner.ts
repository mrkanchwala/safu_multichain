// Splits the runs across workers and cancels stale work, so results stay quick while sliders move.
// The fix line is only shown once it holds at the full run count.
import type { Values } from "../../../sim/engine/settings";
import { LIQ } from "./config";
import { summarise } from "./summary";
import type { RunResult, Summary } from "./summary";

const FIX_STEP_PCT = LIQ.run.fix_step_pct;
const WORKERS = Math.max(2, Math.min(6, (navigator.hardwareConcurrency || 4) - 1));

export interface Result {
  summary: Summary;
  /** undefined = still searching; -1 = investing less doesn't help; otherwise the safe invested share. */
  safeDeployPct?: number;
}

export class Runner {
  private workers: Worker[] = [];
  private gen = 0;

  cancel(): void {
    this.gen += 1;
    this.workers.forEach((w) => w.terminate());
    this.workers = [];
  }

  /** Runs the model; calls back once with the result, and again with the fix if not every run was on time.
   *  A worker crash calls onError once and stops; the next input starts fresh. */
  async run(s: Values, a: Values, onResult: (r: Result) => void, onError: (e: unknown) => void): Promise<void> {
    this.cancel();
    const gen = this.gen;
    try {
      await this.runAndFix(s, a, gen, onResult);
    } catch (e) {
      if (gen === this.gen) onError(e);
    }
  }

  private async runAndFix(s: Values, a: Values, gen: number, onResult: (r: Result) => void): Promise<void> {
    const runs = await this.runAll(s, a);
    if (!runs || gen !== this.gen) return;
    const summary = summarise(s.poolCap, runs);
    if (summary.failedRuns === 0 || s.deployPct <= 0) {
      onResult({ summary, safeDeployPct: -1 });
      return;
    }
    onResult({ summary });
    const safe = await this.verifiedFix(s, a, gen);
    if (safe !== null && gen === this.gen) onResult({ summary, safeDeployPct: safe });
  }

  private spawn(): Worker {
    const w = new Worker(new URL("./liquidity.worker.ts", import.meta.url), { type: "module" });
    this.workers.push(w);
    return w;
  }

  private ask<T>(job: object): Promise<T> {
    return new Promise((resolve, reject) => {
      const w = this.spawn();
      w.onmessage = ({ data }: MessageEvent<T>) => {
        w.terminate();
        resolve(data);
      };
      w.onerror = (e) => {
        w.terminate();
        reject(new Error(`liquidity worker failed: ${e.message}`));
      };
      w.onmessageerror = () => {
        w.terminate();
        reject(new Error("liquidity worker sent an unreadable message"));
      };
      w.postMessage(job);
    });
  }

  /** Every run for one input, split across workers. null if cancelled meanwhile. */
  private async runAll(s: Values, a: Values): Promise<RunResult[] | null> {
    const gen = this.gen;
    const per = Math.ceil(a.runs / WORKERS);
    const jobs: Promise<RunResult[]>[] = [];
    for (let from = 0; from < a.runs; from += per) {
      jobs.push(this.ask<RunResult[]>({ mode: "runs", s, a, from, to: Math.min(a.runs, from + per) }));
    }
    const parts = await Promise.all(jobs);
    return gen === this.gen ? parts.flat() : null;
  }

  /** Quick search in one worker, then step down until every full run is on time. null if cancelled. */
  private async verifiedFix(s: Values, a: Values, gen: number): Promise<number | null> {
    let d = await this.ask<number>({ mode: "fix", s, a });
    while (d >= 0) {
      if (gen !== this.gen) return null;
      const runs = await this.runAll({ ...s, deployPct: d }, a);
      if (!runs) return null;
      if (runs.every((r) => r.short === 0)) return d;
      d -= FIX_STEP_PCT;
    }
    return -1;
  }
}
