import { toDate, type wallet } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Card } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { formatCoins, niceCeiling } from "./format";

/**
 * Net earnings per day (or month) as columns. Hovering, focusing or tapping
 * a column reads it out above the plot; the same numbers are in a table for
 * screen readers, so nothing depends on the pointer or on colour.
 */
export function EarningsChart({ series, bucket }: { series: wallet.EarningsPoint[]; bucket: "day" | "month" }) {
  const { t, i18n } = useTranslation("wallet");
  const [active, setActive] = useState<number | null>(null);

  const values = series.map((point) => Number(point.totals?.net ?? 0n));
  const peak = Math.max(0, ...values);
  const top = niceCeiling(peak);
  const empty = peak === 0;

  const shortDate = (date: Date | undefined) => date?.toLocaleDateString(i18n.language, bucket === "month" ? { month: "short" } : { month: "numeric", day: "numeric" }) ?? "";
  const longDate = (date: Date | undefined) =>
    date?.toLocaleDateString(i18n.language, bucket === "month" ? { year: "numeric", month: "long" } : { month: "long", day: "numeric", weekday: "short" }) ?? "";
  const tick = (value: number) => (value >= 10_000 ? new Intl.NumberFormat(i18n.language, { notation: "compact" }).format(value) : value.toLocaleString("en-US"));

  // The latest bucket is what the readout shows until the person points at another one.
  const shownIndex = active ?? series.length - 1;
  const shown = series[shownIndex];
  // Date labels under the plot: the ends and the middle are enough to place every column.
  const labelled = new Set([0, Math.floor((series.length - 1) / 2), series.length - 1]);

  return (
    <Card className="p-4">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3">
        <h3 className="font-black">{t("earnings.chart.title")}</h3>
        {shown && (
          <p className="text-sm font-bold text-ink-soft" aria-live="polite">
            <span className="font-black text-ink tabular-nums">{formatCoins(shown.totals?.net ?? 0n)}</span>
            <span className="mx-1.5" aria-hidden>
              ·
            </span>
            {longDate(toDate(shown.periodStart))}
            {(shown.totals?.count ?? 0) > 0 && (
              <>
                <span className="mx-1.5" aria-hidden>
                  ·
                </span>
                {t("earnings.reasonCount", { count: shown.totals?.count ?? 0 })}
              </>
            )}
          </p>
        )}
      </div>

      <div className="mt-4 flex gap-2">
        {/* Value axis: three clean ticks. */}
        <div className="flex h-40 w-9 shrink-0 flex-col justify-between text-right text-[10px] leading-none font-bold tabular-nums text-ink-faint">
          <span className="-translate-y-1/2">{tick(top)}</span>
          <span>{tick(top / 2)}</span>
          <span className="translate-y-1/2">0</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="relative h-40">
            <div className="absolute inset-x-0 top-0 border-t border-line" />
            <div className="absolute inset-x-0 top-1/2 border-t border-line" />
            <div className="absolute inset-x-0 bottom-0 border-t border-ink-faint" />
            {empty && <p className="absolute inset-0 grid place-items-center px-4 text-center text-sm font-bold text-ink-faint">{t("earnings.chart.empty")}</p>}
            <div
              role="group"
              // One tab stop for the whole plot; the arrow keys walk the columns.
              tabIndex={0}
              aria-label={t("earnings.chart.keyboard")}
              className="absolute inset-0 flex items-stretch rounded-lg"
              onKeyDown={(event) => {
                if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
                event.preventDefault();
                const from = active ?? series.length - 1;
                setActive(Math.min(series.length - 1, Math.max(0, from + (event.key === "ArrowLeft" ? -1 : 1))));
              }}
              onBlur={() => setActive(null)}
              // A finger leaves the plot as soon as it lifts: keep what was tapped.
              onPointerLeave={(event) => event.pointerType === "mouse" && setActive(null)}
            >
              {series.map((_point, index) => (
                <span
                  // biome-ignore lint/suspicious/noArrayIndexKey: buckets are a fixed sequence in time.
                  key={index}
                  aria-hidden
                  // The whole column is the target, not just the painted bar.
                  className="flex min-w-0 flex-1 items-end justify-center px-px"
                  onPointerEnter={() => setActive(index)}
                  onPointerDown={() => setActive(index)}
                >
                  <span
                    className={cn("block w-full max-w-6 rounded-t-[4px] bg-brand transition-opacity", active !== null && active !== index && "opacity-45")}
                    style={{ height: `${top > 0 ? ((values[index] ?? 0) / top) * 100 : 0}%`, minHeight: (values[index] ?? 0) > 0 ? 2 : 0 }}
                  />
                </span>
              ))}
            </div>
          </div>
          <div className="mt-1.5 flex text-[10px] leading-none font-bold text-ink-faint">
            {series.map((point, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: buckets are a fixed sequence in time.
              <span key={index} className="relative min-w-0 flex-1">
                {labelled.has(index) && (
                  <span className={cn("absolute top-0 whitespace-nowrap", index === 0 ? "left-0" : index === series.length - 1 ? "right-0" : "left-1/2 -translate-x-1/2")}>{shortDate(toDate(point.periodStart))}</span>
                )}
              </span>
            ))}
          </div>
          <div className="h-3" />
        </div>
      </div>

      <table className="sr-only">
        <caption>{t("earnings.chart.title")}</caption>
        <thead>
          <tr>
            <th scope="col">{t("earnings.chart.date")}</th>
            <th scope="col">{t("earnings.totals.net")}</th>
            <th scope="col">{t("earnings.totals.count")}</th>
          </tr>
        </thead>
        <tbody>
          {series.map((point, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: buckets are a fixed sequence in time.
            <tr key={index}>
              <th scope="row">{longDate(toDate(point.periodStart))}</th>
              <td>{formatCoins(point.totals?.net ?? 0n)}</td>
              <td>{point.totals?.count ?? 0}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
  );
}
