// Types for assumptions.js.
import type { SettingDef, Values } from "./settings";
export const ASSUMPTIONS: SettingDef[];
export function assumptionDefaults(): Values;
export function checkAssumptions(values: Values): string[];
