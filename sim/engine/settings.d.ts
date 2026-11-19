// Types for settings.js (the page imports the engine from TypeScript).
export interface SettingDef {
  key: string;
  group: string;
  label: string;
  unit: string;
  def: number;
  min: number;
  max: number;
  step: number;
  liquidity?: boolean;
  tip: string;
}
export type Values = Record<string, number>;
export const SETTINGS: SettingDef[];
export const FIXED: Record<string, number>;
export function defaults(): Values;
export function check(values: Values): string[];
