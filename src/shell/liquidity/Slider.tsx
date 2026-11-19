import type { SettingDef, Values } from "../../../sim/engine/settings";
import { formatValue } from "./model-state";

/** One labelled range input for an engine setting or assumption; the tip shows as a quiet line. */
export function Slider({ def, values, onChange }: { def: SettingDef; values: Values; onChange: (key: string, v: number) => void }) {
  return (
    <div className="liq-row">
      <div className="liq-lab">{def.label}</div>
      <div className="liq-val">{formatValue(values[def.key], def.unit)}</div>
      {def.tip ? <div className="liq-tipline">{def.tip}</div> : null}
      <input
        type="range"
        min={def.min}
        max={def.max}
        step={def.step}
        value={values[def.key]}
        aria-label={def.label}
        onChange={(e) => onChange(def.key, Number(e.target.value))}
      />
    </div>
  );
}
