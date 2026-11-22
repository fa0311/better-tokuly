import { type KeyboardEvent, type PointerEvent, type ReactNode, useState } from "react";
import { cn } from "#/lib/cn";

/** One position along a chart's horizontal axis. */
export interface ChartPoint {
  value: number;
  /** Under the axis (only the ends and the middle are printed). */
  label: string;
  /** In the readout and the table: the full date, or the moment of the video. */
  longLabel: string;
}

/** The smallest "round" number at or above `value` (1, 2, 2.5, 5 or 10 times a power of ten), for the top of an axis. */
export function niceCeiling(value: number): number {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const step = [1, 2, 2.5, 5, 10].find((candidate) => candidate * magnitude >= value) ?? 10;
  return step * magnitude;
}

/** Walks a chart's positions with the arrow keys, Home and End. */
function stepIndex(event: KeyboardEvent, index: number, count: number): number | null {
  const next =
    event.key === "ArrowLeft" || event.key === "ArrowDown"
      ? index - 1
      : event.key === "ArrowRight" || event.key === "ArrowUp"
        ? index + 1
        : event.key === "Home"
          ? 0
          : event.key === "End"
            ? count - 1
            : null;
  return next === null ? null : Math.min(count - 1, Math.max(0, next));
}

function indexUnder(event: PointerEvent<HTMLElement>, count: number): number {
  const box = event.currentTarget.getBoundingClientRect();
  return Math.min(
    count - 1,
    Math.max(0, Math.round(((event.clientX - box.left) / Math.max(1, box.width)) * (count - 1))),
  );
}

/** The card every chart sits in: a title, optionally a headline figure, and the plot. */
export function ChartCard({
  title,
  figure,
  children,
  className,
}: {
  title: string;
  figure?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("sticker rounded-card bg-surface p-4", className)} aria-label={title}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h3 className="font-black">{title}</h3>
        {figure && <p className="text-sm font-bold text-ink-soft">{figure}</p>}
      </div>
      <div className="mt-4">{children}</div>
    </section>
  );
}

function ValueAxis({ top, format }: { top: number; format(value: number): string }) {
  return (
    <div
      className="flex h-44 w-10 shrink-0 flex-col justify-between text-right text-[10px] leading-none font-bold text-ink-faint tabular-nums"
      aria-hidden
    >
      <span className="-translate-y-1/2">{format(top)}</span>
      <span>{format(top / 2)}</span>
      <span className="translate-y-1/2">{format(0)}</span>
    </div>
  );
}

function Gridlines() {
  return (
    <>
      <div className="absolute inset-x-0 top-0 border-t border-line" />
      <div className="absolute inset-x-0 top-1/2 border-t border-line" />
      <div className="absolute inset-x-0 bottom-0 border-t border-ink-faint" />
    </>
  );
}

/** Labels under the plot: the two ends and the middle are enough to place every position. */
function PositionLabels({ points }: { points: ChartPoint[] }) {
  const shown = [...new Set([0, Math.floor((points.length - 1) / 2), points.length - 1])];
  return (
    <div
      className="relative mt-1.5 h-4 text-[10px] leading-none font-bold text-ink-faint"
      aria-hidden
    >
      {shown.map((index) => (
        <span
          key={index}
          className={cn(
            "absolute top-0 whitespace-nowrap",
            index === 0 ? "left-0" : index === points.length - 1 ? "right-0" : "-translate-x-1/2",
          )}
          style={
            index === 0 || index === points.length - 1
              ? undefined
              : { left: `${(index / Math.max(1, points.length - 1)) * 100}%` }
          }
        >
          {points[index]?.label}
        </span>
      ))}
    </div>
  );
}

/** The same numbers as a table, for screen readers and for anyone who does not read charts. */
function DataTable({
  caption,
  positionHeader,
  valueHeader,
  points,
  format,
}: {
  caption: string;
  positionHeader: string;
  valueHeader: string;
  points: ChartPoint[];
  format(value: number): string;
}) {
  return (
    <table className="sr-only">
      <caption>{caption}</caption>
      <thead>
        <tr>
          <th scope="col">{positionHeader}</th>
          <th scope="col">{valueHeader}</th>
        </tr>
      </thead>
      <tbody>
        {points.map((point, index) => (
          <tr key={index}>
            <th scope="row">{point.longLabel}</th>
            <td>{format(point.value)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export interface TrendChartProps {
  /** What is plotted; names the chart for assistive technology and heads the table. */
  title: string;
  points: ChartPoint[];
  /** A value as the axis and the readout print it. */
  format(value: number): string;
  /** The axis tops out here instead of a round number above the peak (1 for shares). */
  max?: number;
  /** Shown over the plot when every value is zero. */
  emptyText: string;
  /** Column heading of the positions in the table ("日付", "再生位置"). */
  positionHeader: string;
  /** How to use the chart from the keyboard, as its accessible name. */
  keyboardLabel: string;
}

/**
 * One series over time (or over the length of a video) as a line above a
 * faint wash. Pointing anywhere in the plot — or walking it with the arrow
 * keys — reads out the nearest position: a hairline finds it, a dot marks it,
 * and its value leads the readout. The same numbers are in a table.
 */
export function TrendChart({
  title,
  points,
  format,
  max,
  emptyText,
  positionHeader,
  keyboardLabel,
}: TrendChartProps) {
  const [active, setActive] = useState<number | null>(null);
  const peak = Math.max(0, ...points.map((point) => point.value));
  const top = max ?? niceCeiling(peak);
  const empty = peak === 0;
  const count = points.length;
  const x = (index: number) => (count <= 1 ? 50 : (index / (count - 1)) * 100);
  const y = (value: number) => 100 - Math.min(100, (value / top) * 100);
  const line = points
    .map(
      (point, index) =>
        `${index === 0 ? "M" : "L"}${x(index).toFixed(2)},${y(point.value).toFixed(2)}`,
    )
    .join(" ");
  const shown = active ?? count - 1;
  const point = points[shown];

  return (
    <div>
      <div className="flex gap-2">
        <ValueAxis top={top} format={format} />
        <div className="min-w-0 flex-1">
          <div
            role="slider"
            tabIndex={0}
            aria-label={keyboardLabel}
            aria-valuemin={0}
            aria-valuemax={Math.max(0, count - 1)}
            aria-valuenow={shown}
            aria-valuetext={point ? `${point.longLabel}: ${format(point.value)}` : undefined}
            data-testid="trend-chart"
            className="relative h-44 touch-pan-y rounded-lg"
            onKeyDown={(event) => {
              const next = stepIndex(event, shown, count);
              if (next === null) return;
              event.preventDefault();
              setActive(next);
            }}
            onBlur={() => setActive(null)}
            onPointerMove={(event) => setActive(indexUnder(event, count))}
            onPointerDown={(event) => setActive(indexUnder(event, count))}
            // A finger leaves the plot as soon as it lifts: keep what was tapped.
            onPointerLeave={(event) => event.pointerType === "mouse" && setActive(null)}
          >
            <Gridlines />
            {empty ? (
              <p className="absolute inset-0 grid place-items-center px-4 text-center text-sm font-bold text-ink-faint">
                <span className="bg-surface px-2">{emptyText}</span>
              </p>
            ) : (
              <>
                <svg
                  className="absolute inset-0 size-full overflow-visible"
                  viewBox="0 0 100 100"
                  preserveAspectRatio="none"
                  aria-hidden
                >
                  <path
                    d={`${line} L${x(count - 1)},100 L${x(0)},100 Z`}
                    className="fill-brand opacity-10"
                  />
                  <path
                    d={line}
                    fill="none"
                    strokeWidth={2}
                    strokeLinejoin="round"
                    strokeLinecap="round"
                    vectorEffect="non-scaling-stroke"
                    className="stroke-brand"
                  />
                </svg>
                {point && (
                  <>
                    {active !== null && (
                      <span
                        aria-hidden
                        className="absolute inset-y-0 w-px bg-ink-faint"
                        style={{ left: `${x(shown)}%` }}
                      />
                    )}
                    <span
                      aria-hidden
                      className="absolute size-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-surface bg-brand"
                      style={{ left: `${x(shown)}%`, top: `${y(point.value)}%` }}
                    />
                  </>
                )}
              </>
            )}
            {point && active !== null && !empty && (
              <div
                aria-hidden
                className={cn(
                  "pointer-events-none absolute top-1 z-10 rounded-xl border-2 border-ink bg-surface px-2.5 py-1.5 text-left whitespace-nowrap shadow-pop-sm",
                  x(shown) > 55 ? "-translate-x-full" : "",
                )}
                style={{ left: `calc(${x(shown)}% + ${x(shown) > 55 ? -8 : 8}px)` }}
              >
                <p className="text-sm leading-tight font-black">{format(point.value)}</p>
                <p className="text-[11px] leading-tight font-bold text-ink-soft">
                  {point.longLabel}
                </p>
              </div>
            )}
          </div>
          <PositionLabels points={points} />
        </div>
      </div>
      <DataTable
        caption={title}
        positionHeader={positionHeader}
        valueHeader={title}
        points={points}
        format={format}
      />
    </div>
  );
}

export interface ColumnChartProps {
  title: string;
  points: ChartPoint[];
  format(value: number): string;
  emptyText: string;
  positionHeader: string;
  keyboardLabel: string;
  /** Height of the plot; realtime strips are shorter than the main charts. */
  compact?: boolean;
}

/**
 * Counts per bucket (an hour, a minute) as thin columns on one baseline.
 * Each column answers the pointer with its bucket and value; the arrow keys
 * walk the buckets; the numbers are in a table as well.
 */
export function ColumnChart({
  title,
  points,
  format,
  emptyText,
  positionHeader,
  keyboardLabel,
  compact,
}: ColumnChartProps) {
  const [active, setActive] = useState<number | null>(null);
  const peak = Math.max(0, ...points.map((point) => point.value));
  const top = niceCeiling(peak);
  const empty = peak === 0;
  const shown = active ?? points.length - 1;
  const point = points[shown];

  return (
    <div>
      <p className="mb-2 h-5 truncate text-sm font-bold text-ink-soft" aria-hidden>
        {point && (
          <>
            <span className="font-black text-ink">{format(point.value)}</span>
            <span className="mx-1.5">·</span>
            {point.longLabel}
          </>
        )}
      </p>
      <div
        role="slider"
        tabIndex={0}
        aria-label={keyboardLabel}
        aria-valuemin={0}
        aria-valuemax={Math.max(0, points.length - 1)}
        aria-valuenow={shown}
        aria-valuetext={point ? `${point.longLabel}: ${format(point.value)}` : undefined}
        data-testid="column-chart"
        className={cn("relative flex items-stretch rounded-lg", compact ? "h-20" : "h-32")}
        onKeyDown={(event) => {
          const next = stepIndex(event, shown, points.length);
          if (next === null) return;
          event.preventDefault();
          setActive(next);
        }}
        onBlur={() => setActive(null)}
        onPointerLeave={(event) => event.pointerType === "mouse" && setActive(null)}
      >
        <div className="pointer-events-none absolute inset-x-0 bottom-0 border-t border-ink-faint" />
        {empty && (
          <p className="pointer-events-none absolute inset-0 grid place-items-center px-4 text-center text-sm font-bold text-ink-faint">
            <span className="bg-surface px-2">{emptyText}</span>
          </p>
        )}
        {points.map((bucket, index) => (
          // The whole slot is the target, not just the painted column; the gap between columns is the surface showing through.
          <span
            key={index}
            aria-hidden
            className="flex min-w-0 flex-1 items-end justify-center px-px"
            onPointerEnter={() => setActive(index)}
            onPointerDown={() => setActive(index)}
          >
            <span
              className={cn(
                "block w-full max-w-6 rounded-t-[4px] bg-brand transition-opacity",
                active !== null && active !== index && "opacity-45",
              )}
              style={{
                height: `${(bucket.value / top) * 100}%`,
                minHeight: bucket.value > 0 ? 2 : 0,
              }}
            />
          </span>
        ))}
      </div>
      <PositionLabels points={points} />
      <DataTable
        caption={title}
        positionHeader={positionHeader}
        valueHeader={title}
        points={points}
        format={format}
      />
    </div>
  );
}

export interface RankedBar {
  key: string;
  label: string;
  value: number;
  /** Printed at the tip of the bar: the value, usually with its share. */
  valueText: string;
}

/**
 * Parts of a whole compared with each other: one horizontal bar per part on
 * a shared baseline, largest first, all in one hue, each with its name and
 * its value in text.
 */
export function RankedBars({
  title,
  bars,
  emptyText,
}: {
  title: string;
  bars: RankedBar[];
  emptyText: string;
}) {
  const peak = Math.max(0, ...bars.map((bar) => bar.value));
  if (bars.length === 0 || peak === 0)
    return <p className="py-6 text-center text-sm font-bold text-ink-faint">{emptyText}</p>;
  return (
    <ul className="flex flex-col gap-3" aria-label={title} data-testid="ranked-bars">
      {[...bars]
        .sort((a, b) => b.value - a.value)
        .map((bar) => (
          <li key={bar.key}>
            <div className="flex items-baseline justify-between gap-3 text-sm">
              <span className="min-w-0 truncate font-bold">{bar.label}</span>
              <span className="shrink-0 font-black tabular-nums">{bar.valueText}</span>
            </div>
            <div className="mt-1 h-3 rounded-r-[4px] bg-surface-2" aria-hidden>
              <div
                className="h-full rounded-r-[4px] bg-brand"
                style={{ width: `${(bar.value / peak) * 100}%`, minWidth: bar.value > 0 ? 3 : 0 }}
              />
            </div>
          </li>
        ))}
    </ul>
  );
}

/** A headline number with its name, and optionally a second line that puts it in context. */
export function StatTile({
  label,
  value,
  detail,
  className,
}: {
  label: string;
  value: string;
  detail?: ReactNode;
  className?: string;
}) {
  return (
    <div
      data-testid="stat-tile"
      className={cn("sticker-sm rounded-2xl bg-surface p-3.5", className)}
    >
      <p className="truncate text-xs font-bold text-ink-soft">{label}</p>
      <p className="mt-1 truncate text-2xl leading-tight font-black">{value}</p>
      {detail && <p className="mt-0.5 truncate text-xs font-bold text-ink-faint">{detail}</p>}
    </div>
  );
}
