import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { fmtNum } from "../lib/format";

function niceMax(v: number): number {
  if (v <= 0) return 1;
  const exp = Math.pow(10, Math.floor(Math.log10(v)));
  const n = v / exp;
  const step = n <= 1 ? 1 : n <= 2 ? 2 : n <= 5 ? 5 : 10;
  return step * exp;
}

/** Path for a column with a 4px rounded top and a square base. */
function columnPath(x: number, y: number, w: number, h: number): string {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

export type ColumnDatum = { key: string; label: string; value: number; tooltip: ReactNode };

/** Single-series column chart with hover tooltips. */
export function ColumnChart({ data, height = 200, ariaLabel }: { data: ColumnDatum[]; height?: number; ariaLabel: string }) {
  const [hover, setHover] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(200, Math.round(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const pad = { top: 10, right: 8, bottom: 22, left: 36 };
  const max = niceMax(Math.max(0, ...data.map((d) => d.value)));
  const ticks = [0, max / 2, max];
  const innerW = width - pad.left - pad.right;
  const innerH = height - pad.top - pad.bottom;
  const slot = data.length ? innerW / data.length : innerW;
  const barW = Math.max(2, Math.min(24, slot - 2));
  // Keep at least ~64px per axis label so they never overlap.
  const maxLabels = Math.max(2, Math.floor(innerW / 64));
  const labelEvery = Math.max(1, Math.ceil(data.length / maxLabels));

  const hovered = hover != null ? data[hover] : null;
  const hx = hover != null ? pad.left + slot * hover + slot / 2 : 0;

  return (
    <div className="chart" ref={ref}>
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={ariaLabel} width={width} height={height}>
        <g className="grid">
          {ticks.map((t) => {
            const y = pad.top + innerH - (t / max) * innerH;
            return <line key={t} x1={pad.left} x2={width - pad.right} y1={y} y2={y} />;
          })}
        </g>
        <g className="axis">
          {ticks.map((t) => {
            const y = pad.top + innerH - (t / max) * innerH;
            return (
              <text key={t} x={pad.left - 6} y={y + 4} textAnchor="end">
                {fmtNum(t)}
              </text>
            );
          })}
          {data.map((d, i) =>
            i % labelEvery === 0 ? (
              <text key={d.key} x={pad.left + slot * i + slot / 2} y={height - 6} textAnchor="middle">
                {d.label}
              </text>
            ) : null,
          )}
        </g>
        {data.map((d, i) => {
          const h = (d.value / max) * innerH;
          const x = pad.left + slot * i + (slot - barW) / 2;
          return (
            <g key={d.key}>
              {h > 0 && <path className={`bar${hover === i ? " hover" : ""}`} d={columnPath(x, pad.top + innerH - h, barW, h)} />}
              <rect
                className="hit"
                x={pad.left + slot * i}
                y={pad.top}
                width={slot}
                height={innerH}
                onMouseEnter={() => setHover(i)}
                onMouseLeave={() => setHover(null)}
              />
            </g>
          );
        })}
      </svg>
      {hovered && (
        <div className="tooltip" style={{ left: hx, top: pad.top + innerH * 0.35 }}>
          {hovered.tooltip}
        </div>
      )}
    </div>
  );
}

export type HBarDatum = { key: string | number; label: ReactNode; value: number; display: string; onClick?: () => void; title?: string };

/** Horizontal single-series bars with the value at the tip. */
export function HBars({ data }: { data: HBarDatum[] }) {
  const max = useMemo(() => Math.max(0, ...data.map((d) => d.value)) || 1, [data]);
  return (
    <div className="hbars">
      {data.map((d) => (
        <div className="hbar" key={d.key} title={d.title}>
          <div className="ellipsis">{d.onClick ? <a onClick={d.onClick}>{d.label}</a> : d.label}</div>
          <div className="track">
            <div className="fill" style={{ width: `${(d.value / max) * 100}%` }} />
          </div>
          <div className="val">{d.display}</div>
        </div>
      ))}
    </div>
  );
}

/** 0–10 score bar used for rubric criteria. */
export function ScoreBar({ score }: { score: number | null | undefined }) {
  const v = Math.max(0, Math.min(10, score ?? 0));
  return (
    <div className="bar-track" role="meter" aria-valuemin={0} aria-valuemax={10} aria-valuenow={v}>
      <div className="bar-fill" style={{ width: `${v * 10}%` }} />
    </div>
  );
}
