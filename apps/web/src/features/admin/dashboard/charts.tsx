import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

/**
 * The dashboard's charts: plain SVG, one hue. Every bar sits in a row with
 * its name and its figure, so the list is its own table view and nothing
 * depends on colour or on hovering.
 */

export interface BarDatum {
  key: string;
  label: string;
  value: number;
  /** The figure printed at the end of the row. */
  display: string;
  /** Detail for the hover tooltip (a share, a caveat). */
  detail?: string;
}

const BAR_HEIGHT = 10;
const BAR_RADIUS = 4;

/**
 * A horizontal bar: square at the baseline (left), rounded at the data end.
 * With `track` it is a meter — the unfilled part is a lighter step of the
 * same hue and stands for the whole the value is measured against.
 */
function Bar({ fraction, track }: { fraction: number; track?: boolean }) {
  const percent = Math.max(0, Math.min(1, fraction)) * 100;
  return (
    <svg className="block w-full" height={BAR_HEIGHT} aria-hidden>
      {track && (
        <rect width="100%" height={BAR_HEIGHT} rx={BAR_RADIUS} className="fill-brand-soft" />
      )}
      {percent > 0 && (
        <g className="fill-brand transition-[fill] group-hover/bar:fill-brand-strong">
          <rect width={`${percent}%`} height={BAR_HEIGHT} rx={BAR_RADIUS} />
          {/* Squares off the rounding at the baseline; a meter keeps its track's rounding. */}
          {!track && (
            <rect height={BAR_HEIGHT} style={{ width: `min(${percent}%, ${BAR_RADIUS}px)` }} />
          )}
        </g>
      )}
    </svg>
  );
}

/**
 * Ranked bars (`max` omitted: the longest bar spans the width) or a meter
 * list (`max` given: each bar is measured against that whole).
 */
export function BarList({
  label,
  data,
  max,
  empty,
}: {
  label: string;
  data: BarDatum[];
  max?: number;
  empty?: ReactNode;
}) {
  const scale = max ?? Math.max(0, ...data.map((datum) => datum.value));
  if (data.length === 0) return empty ?? null;
  return (
    <ul aria-label={label} className="flex flex-col">
      {data.map((datum) => (
        <li
          key={datum.key}
          title={datum.detail}
          className="group/bar grid grid-cols-[minmax(0,9rem)_minmax(2rem,1fr)_auto] items-center gap-3 rounded-lg px-4 py-1.5 text-sm hover:bg-surface-2"
        >
          <span className="truncate text-ink-soft">{datum.label}</span>
          <Bar fraction={scale > 0 ? datum.value / scale : 0} track={max !== undefined} />
          <span className="min-w-12 text-right font-bold tabular-nums">{datum.display}</span>
        </li>
      ))}
    </ul>
  );
}

/** A headline number with its label; `hero` is the one figure the page leads with. */
export function StatTile({
  label,
  value,
  detail,
  hero,
  tone,
  className,
}: {
  label: string;
  value: string;
  detail?: ReactNode;
  hero?: boolean /** Marks a figure that needs attention. */;
  tone?: "alert";
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-col justify-between gap-1 rounded-2xl border-2 border-line bg-surface p-4",
        tone === "alert" && "border-danger",
        className,
      )}
    >
      <p className="text-xs font-bold text-ink-soft">{label}</p>
      <p className={cn("truncate font-black leading-none", hero ? "text-5xl" : "text-2xl")}>
        {value}
      </p>
      {detail && <p className="truncate text-xs text-ink-faint">{detail}</p>}
    </div>
  );
}
