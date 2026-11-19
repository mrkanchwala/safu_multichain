import { useEffect, useRef, useState } from "react";
import { DAYS_PER_YEAR as YEAR } from "../../../sim/engine/model";
import type { StressBand } from "./model-state";
import type { ChartPoint } from "./summary";

// Two thin lines, labels at the line ends, a baseline only (F20 page, founder 19 Nov: no clutter).
// Narrow screens get a shorter chart and a slimmer label column, drawn at true size (no scaling).
const NARROW = 520;
const size = (w: number) => (w < NARROW ? { HEIGHT: 220, right: 104 } : { HEIGHT: 280, right: 150 });
const LABEL_GAP = 38; // keep the two end labels from overlapping

const money = (v: number) => (v >= 1e6 ? `$${(v / 1e6).toFixed(1)}M` : `$${Math.round(v / 1000)}K`);

function useWidth<T extends HTMLElement>(): [React.RefObject<T | null>, number] {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(800);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width];
}

export function LiquidityChart({ data, bands, title }: { data: ChartPoint[]; bands: StressBand[]; title: string }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<ChartPoint | null>(null);
  const W = Math.max(width, 280);
  const { HEIGHT, right } = size(W);
  const PAD = { left: 4, right, top: 12, bottom: 26 };
  const maxDay = data[data.length - 1].day;
  const maxY = Math.max(...data.map((d) => Math.max(d.held, d.owed))) * 1.05 || 1;
  const x = (day: number) => PAD.left + (day / maxDay) * (W - PAD.left - PAD.right);
  const y = (v: number) => PAD.top + (1 - v / maxY) * (HEIGHT - PAD.top - PAD.bottom);
  const path = (k: "held" | "owed") => data.map((d, i) => `${i ? "L" : "M"}${x(d.day).toFixed(1)},${y(d[k]).toFixed(1)}`).join("");
  const last = data[data.length - 1];
  const owedY = Math.min(y(last.owed), HEIGHT - PAD.bottom - 22);
  const heldY = Math.abs(y(last.held) - owedY) < LABEL_GAP ? owedY - LABEL_GAP : y(last.held);
  const years = Array.from({ length: Math.floor(maxDay / YEAR) }, (_, i) => (i + 1) * YEAR);

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    const box = e.currentTarget.ownerSVGElement?.getBoundingClientRect();
    if (!box) return;
    const day = (((e.clientX - box.left) * (W / box.width) - PAD.left) / (W - PAD.left - PAD.right)) * maxDay;
    setHover(data.reduce((a, b) => (Math.abs(b.day - day) < Math.abs(a.day - day) ? b : a)));
  };

  return (
    <div className="liq-chart" ref={ref}>
      <svg viewBox={`0 0 ${W} ${HEIGHT}`} height={HEIGHT} role="img" aria-label={title}>
        {bands.map((b) => (
          <rect key={b.from} className="liq-band" x={x(b.from)} y={PAD.top} width={Math.max(x(b.to) - x(b.from), 3)} height={HEIGHT - PAD.top - PAD.bottom} />
        ))}
        <line className="liq-base" x1={PAD.left} x2={W - PAD.right} y1={HEIGHT - PAD.bottom} y2={HEIGHT - PAD.bottom} />
        <text x={PAD.left} y={HEIGHT - 6}>Start</text>
        {years.map((d) => (
          <text key={d} x={x(d)} y={HEIGHT - 6} textAnchor={d + YEAR > maxDay ? "end" : "middle"}>
            Year {d / YEAR}
          </text>
        ))}
        <path className="liq-held" d={path("held")} />
        <path className="liq-owed" d={path("owed")} />
        <text className="liq-lbl" x={W - PAD.right + 12} y={heldY - 2}>In the pool</text>
        <text x={W - PAD.right + 12} y={heldY + 15}>{money(last.held)}</text>
        <text className="liq-lbl" x={W - PAD.right + 12} y={owedY - 2}>Owed on claims</text>
        <text x={W - PAD.right + 12} y={owedY + 15}>{money(last.owed)}</text>
        {hover ? <line className="liq-cross" x1={x(hover.day)} x2={x(hover.day)} y1={PAD.top} y2={HEIGHT - PAD.bottom} /> : null}
        <rect
          className="liq-hit"
          x={PAD.left}
          y={PAD.top}
          width={W - PAD.left - PAD.right}
          height={HEIGHT - PAD.top - PAD.bottom}
          onPointerMove={onMove}
          onPointerLeave={() => setHover(null)}
        />
      </svg>
      {hover ? (
        <div className="liq-tip" style={{ left: Math.min((x(hover.day) / W) * width + 12, width - 190) }}>
          Day {hover.day}
          <br />
          In the pool <b>{money(hover.held)}</b>
          <br />
          Owed on claims <b>{money(hover.owed)}</b>
          {hover.line ? (
            <>
              <br />
              Waiting in line <b>{hover.line}</b>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
