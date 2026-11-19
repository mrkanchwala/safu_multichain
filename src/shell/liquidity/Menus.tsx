// The tap-to-change words in the page's sentences, and the small menus they open.
import { useLayoutEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import type { Values } from "../../../sim/engine/settings";
import { LIQ } from "./config";
import { CUSTOM, fillCopy, rateWord, settingDef } from "./model-state";
import type { PageState } from "./model-state";
import { Slider } from "./Slider";

const POP_WIDTH = 360;

/** A word in the sentence that opens a menu under itself. Clicks inside the menu never close it. */
export function Tap(props: {
  label: string;
  open: boolean;
  onToggle: () => void;
  story: RefObject<HTMLDivElement | null>;
  children: ReactNode;
}) {
  const btn = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState({ left: 0, top: 0 });
  useLayoutEffect(() => {
    const box = props.story.current?.getBoundingClientRect();
    const b = btn.current?.getBoundingClientRect();
    if (!props.open || !box || !b) return;
    setPos({ left: Math.max(0, Math.min(b.left - box.left, box.width - POP_WIDTH)), top: b.bottom - box.top + 8 });
  }, [props.open, props.story]);
  return (
    <>
      {/* A span, not a button: a button is one unbreakable box, so a long label broke the sentence (19 Nov). */}
      <span
        ref={btn}
        role="button"
        tabIndex={0}
        className="liq-tap"
        aria-expanded={props.open}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        onKeyDown={(e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          e.preventDefault();
          e.stopPropagation();
          props.onToggle();
        }}
      >
        {props.label}
      </span>
      {props.open ? (
        <div className="liq-pop" style={pos} onClick={(e) => e.stopPropagation()}>
          {props.children}
        </div>
      ) : null}
    </>
  );
}

export function RateMenu({ st, onPick, onCustom }: { st: PageState; onPick: (key: string, rate?: number) => void; onCustom: (v: number) => void }) {
  const options = [...LIQ.years, { key: CUSTOM, label: "Custom", rate: undefined, tip: `Up to ${LIQ.run.custom_max_pct}% a year.` }];
  return (
    <>
      {options.map((y) => (
        <button key={y.key} className="liq-opt" role="menuitemradio" aria-checked={st.year === y.key} onClick={() => onPick(y.key, y.rate)}>
          <div className="liq-opt-t">
            {y.label}
            <span>{y.rate === undefined ? rateWord(st) : `${y.rate}%`}</span>
          </div>
          <div className="liq-opt-s">{y.tip}</div>
        </button>
      ))}
      {st.year === CUSTOM ? (
        <input
          type="range"
          min={0}
          max={LIQ.run.custom_max_pct}
          step={0.05}
          value={st.a.theftPctYear}
          aria-label="Stakers robbed per year"
          onChange={(e) => onCustom(Number(e.target.value))}
        />
      ) : null}
    </>
  );
}

export function WhatMenu({ st, onSwitch }: { st: PageState; onSwitch: (key: string, on: boolean) => void }) {
  return (
    <>
      {LIQ.switches.map((w) => (
        <label key={w.key} className="liq-opt">
          <div className="liq-opt-t">
            <span className="liq-check">
              <input type="checkbox" checked={!!st.on[w.key]} onChange={(e) => onSwitch(w.key, e.target.checked)} />
              {fillCopy(w.label, w)}
            </span>
          </div>
          <div className="liq-opt-s">{fillCopy(w.tip, w)}</div>
        </label>
      ))}
    </>
  );
}

/** Sliders inside the revenue menus reuse the engine's own setting definitions. */
export function SliderMenu({ keys, values, onChange }: { keys: string[]; values: Values; onChange: (key: string, v: number) => void }) {
  return (
    <>
      {keys.map((k) => (
        <Slider key={k} def={settingDef(k)} values={values} onChange={onChange} />
      ))}
    </>
  );
}
