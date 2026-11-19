// The Liquidity page's copy and sourced inputs from hq/liquidity.toml, baked in at build time
// (vite.config.ts checks every key). Pool rules come from the engine, never from here.
import type { Values } from "../../../sim/engine/settings";

export interface YearPreset {
  key: string;
  label: string;
  rate: number;
  tip: string;
}

export interface WhatIf {
  key: string;
  phrase: string;
  label: string;
  tip: string;
  on: Values;
}

export interface LiquidityConfig {
  run: {
    runs: number;
    custom_max_pct: number;
    fix_step_pct: number;
    fix_quick_runs: number;
    tier_a_pct: number;
    tier_b_pct: number;
    hidden_settings: string[];
    hidden_assumptions: string[];
  };
  copy: {
    title: string;
    chart_title: string;
    settings_note: string;
    audit_note: string;
    revenue_note: string;
    link_label: string;
  };
  years: YearPreset[];
  switches: WhatIf[];
}

declare const __LIQ__: LiquidityConfig;

export const LIQ: LiquidityConfig = __LIQ__;
