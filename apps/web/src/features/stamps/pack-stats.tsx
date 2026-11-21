import { useQuery } from "@tanstack/react-query";
import type { stamp } from "@tokuly/api";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Coins } from "#/components/ui/coins";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { contextKey, formatCount, stampKeys, studioClient } from "./api";
import { StampImage } from "./stamp-image";

const RANGES = ["7", "30", "90"] as const;
type Range = (typeof RANGES)[number];
const METRICS = ["sends", "installs", "sales", "earnings"] as const;
type Metric = (typeof METRICS)[number];

/** The top of a value axis: the next "clean" number at or above `value` (1, 2 or 5 times a power of ten). */
export function niceCeiling(value: number): number {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const leading = value / magnitude;
  return (leading <= 1 ? 1 : leading <= 2 ? 2 : leading <= 5 ? 5 : 10) * magnitude;
}

/** A pack's numbers for its creator: totals, a day-by-day chart, where it is sent, and its most sent stamps. */
export function PackStats({ packId, paid }: { packId: string; /** Paid packs also show sales and earnings. */ paid: boolean }) {
  const { t } = useTranslation("stamps");
  const [range, setRange] = useState<Range>("30");
  const [metric, setMetric] = useState<Metric>("sends");
  const days = Number(range);
  const { data, isPending, isPlaceholderData } = useQuery({
    queryKey: stampKeys.packStats(packId, days),
    queryFn: () => studioClient().getPackStats({ packId, days }),
    // Changing the range keeps the previous numbers on screen until the new ones arrive.
    placeholderData: (previous) => previous,
  });

  if (isPending || !data) return <Skeleton className="h-72" />;
  const metrics: readonly Metric[] = paid || data.saleCount > 0n ? METRICS : ["sends", "installs"];
  const shownMetric = metrics.includes(metric) ? metric : "sends";
  const contexts = [...data.contexts].filter((item) => item.sends > 0n).sort((a, b) => Number(b.sends - a.sends));
  const mostSent = Number(data.topStamps[0]?.sends ?? 0n);

  return (
    <div className={cn("flex flex-col gap-4 transition-opacity", isPlaceholderData && "opacity-60")}>
      <dl className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <Total label={t("stats.installs")} value={formatCount(data.installCount)} />
        <Total label={t("stats.sends")} value={formatCount(data.sendCount)} />
        {metrics.includes("sales") && (
          <>
            <Total label={t("stats.sales")} value={formatCount(data.saleCount)} />
            <Total label={t("stats.earnings")} value={<Coins amount={data.earnings} />} />
          </>
        )}
        <Total label={t("stats.likes")} value={formatCount(data.likeCount)} />
      </dl>

      <div className="flex flex-wrap items-center gap-2">
        <Tabs<Range> value={range} onChange={setRange} items={RANGES.map((value) => ({ value, label: t("stats.days", { count: Number(value) }) }))} />
        <Tabs<Metric> value={shownMetric} onChange={setMetric} items={metrics.map((value) => ({ value, label: t(`stats.${value}`) }))} />
      </div>
      <DailyChart daily={data.daily} metric={shownMetric} />

      <div className="grid gap-4 md:grid-cols-2">
        <section className="rounded-2xl border-2 border-line p-4">
          <h3 className="pb-3 font-black">{t("stats.contexts")}</h3>
          {contexts.length === 0 ? (
            <p className="py-4 text-center text-sm font-bold text-ink-faint">{t("stats.noSends")}</p>
          ) : (
            <ul className="flex flex-col gap-2.5">
              {contexts.map((item) => (
                <RankedBar key={item.context} label={t(contextKey(item.context))} value={Number(item.sends)} peak={Number(contexts[0]!.sends)} />
              ))}
            </ul>
          )}
        </section>
        <section className="rounded-2xl border-2 border-line p-4">
          <h3 className="pb-3 font-black">{t("stats.topStamps")}</h3>
          {data.topStamps.length === 0 || mostSent === 0 ? (
            <p className="py-4 text-center text-sm font-bold text-ink-faint">{t("stats.noSends")}</p>
          ) : (
            <ol className="flex flex-col gap-2.5">
              {data.topStamps.map(
                (item) =>
                  item.stamp?.stampRef && (
                    <RankedBar
                      key={item.stamp.stampRef.id}
                      label={item.stamp.stampRef.name || t("stamp.unnamed")}
                      value={Number(item.sends)}
                      peak={mostSent}
                      leading={<StampImage stamp={item.stamp.stampRef} image={item.stamp.image} size={36} play="never" />}
                    />
                  ),
              )}
            </ol>
          )}
        </section>
      </div>
    </div>
  );
}

function Total({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="rounded-2xl bg-surface-2 px-3 py-2.5">
      <dt className="text-xs font-bold text-ink-soft">{label}</dt>
      <dd className="text-xl font-black">{value}</dd>
    </div>
  );
}

/** One row of a ranked list: a name, a bar on a shared baseline, and the value at its tip. */
function RankedBar({ label, value, peak, leading }: { label: string; value: number; peak: number; leading?: ReactNode }) {
  return (
    <li className="flex items-center gap-2.5">
      {leading}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2 text-sm">
          <span className="truncate font-bold">{label}</span>
          <span className="shrink-0 font-black tabular-nums">{value.toLocaleString("en-US")}</span>
        </div>
        <div className="mt-1 h-2">
          <div className="h-full min-w-1 rounded-r-[4px] bg-brand" style={{ width: `${peak > 0 ? (value / peak) * 100 : 0}%` }} />
        </div>
      </div>
    </li>
  );
}

function metricValue(day: stamp.DailyPackStats, metric: Metric): number {
  return Number(metric === "sends" ? day.sends : metric === "installs" ? day.installs : metric === "sales" ? day.sales : day.earnings);
}

/**
 * One measure per day as columns. Pointing at, tapping or arrowing to a
 * column reads it out above the plot; the same numbers are in a table for
 * screen readers, so nothing depends on the pointer or on colour.
 */
function DailyChart({ daily, metric }: { daily: stamp.DailyPackStats[]; metric: Metric }) {
  const { t, i18n } = useTranslation("stamps");
  const [active, setActive] = useState<number | null>(null);
  const values = daily.map((day) => metricValue(day, metric));
  // New installs are net of same-day uninstalls and can dip below zero; the columns start at zero.
  const peak = Math.max(0, ...values);
  const top = niceCeiling(peak);
  const total = values.reduce((sum, value) => sum + value, 0);

  // Dates are calendar days in Japan time; noon keeps them on the same day in any time zone.
  const dateOf = (day: stamp.DailyPackStats) => new Date(`${day.date}T12:00:00+09:00`);
  const short = (day: stamp.DailyPackStats) => dateOf(day).toLocaleDateString(i18n.language, { month: "numeric", day: "numeric", timeZone: "Asia/Tokyo" });
  const long = (day: stamp.DailyPackStats) => dateOf(day).toLocaleDateString(i18n.language, { month: "long", day: "numeric", weekday: "short", timeZone: "Asia/Tokyo" });
  const tick = (value: number) => (value >= 10_000 ? formatCount(value) : value.toLocaleString("en-US"));

  const shown = active === null ? undefined : daily[active];
  // What a screen reader hears for the slider's position: the pointed-at day, or the latest one.
  const spoken = shown ?? daily[daily.length - 1];
  const labelled = new Set([0, Math.floor((daily.length - 1) / 2), daily.length - 1]);

  return (
    <figure className="rounded-2xl border-2 border-line p-4">
      <figcaption className="flex flex-wrap items-baseline justify-between gap-x-3">
        <span className="font-black">{t("stats.chartTitle", { metric: t(`stats.${metric}`) })}</span>
        <span className="text-sm font-bold text-ink-soft" aria-live="polite">
          <span className="font-black text-ink tabular-nums">{(shown ? metricValue(shown, metric) : total).toLocaleString("en-US")}</span>
          <span className="mx-1.5" aria-hidden>
            ·
          </span>
          {shown ? long(shown) : t("stats.periodTotal", { count: daily.length })}
        </span>
      </figcaption>

      <div className="mt-4 flex gap-2">
        <div className="flex h-36 w-9 shrink-0 flex-col justify-between text-right text-[10px] leading-none font-bold text-ink-faint tabular-nums">
          <span className="-translate-y-1/2">{tick(top)}</span>
          {/* Counts are whole numbers: no "0.5" on a scale that only reaches 1. */}
          <span>{Number.isInteger(top / 2) ? tick(top / 2) : ""}</span>
          <span className="translate-y-1/2">0</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="relative h-36">
            <div className="absolute inset-x-0 top-0 border-t border-line" />
            <div className="absolute inset-x-0 top-1/2 border-t border-line" />
            <div className="absolute inset-x-0 bottom-0 border-t border-ink-faint" />
            {peak === 0 && (
              <p className="absolute inset-0 grid place-items-center px-4 text-center text-sm font-bold text-ink-faint">
                {/* A patch of surface under the words, so the gridline does not strike them through. */}
                <span className="bg-bg px-2">{t("stats.chartEmpty")}</span>
              </p>
            )}
            <div
              // For the keyboard the plot is a slider over its days: one tab stop, the arrow
              // keys walk the columns, and each stop is announced as "date: value".
              role="slider"
              tabIndex={0}
              aria-label={t("stats.chartKeyboard")}
              aria-valuemin={0}
              aria-valuemax={Math.max(0, daily.length - 1)}
              aria-valuenow={active ?? Math.max(0, daily.length - 1)}
              aria-valuetext={spoken ? `${long(spoken)}: ${metricValue(spoken, metric).toLocaleString("en-US")}` : undefined}
              className="absolute inset-0 flex items-stretch rounded-lg"
              onKeyDown={(event) => {
                if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
                event.preventDefault();
                const from = active ?? daily.length - 1;
                setActive(Math.min(daily.length - 1, Math.max(0, from + (event.key === "ArrowLeft" ? -1 : 1))));
              }}
              onBlur={() => setActive(null)}
              // A finger leaves the plot as soon as it lifts: keep what was tapped.
              onPointerLeave={(event) => event.pointerType === "mouse" && setActive(null)}
            >
              {daily.map((day, index) => (
                <span
                  key={day.date}
                  aria-hidden
                  // The whole column is the target, not just the painted bar.
                  className="flex min-w-0 flex-1 items-end justify-center px-px"
                  onPointerEnter={() => setActive(index)}
                  onPointerDown={() => setActive(index)}
                >
                  <span
                    className={cn("block w-full max-w-6 rounded-t-[4px] bg-brand transition-opacity", active !== null && active !== index && "opacity-45")}
                    style={{ height: `${Math.max(0, values[index] ?? 0) / top * 100}%`, minHeight: (values[index] ?? 0) > 0 ? 2 : 0 }}
                  />
                </span>
              ))}
            </div>
          </div>
          <div className="mt-1.5 flex h-3 text-[10px] leading-none font-bold text-ink-faint">
            {daily.map((day, index) => (
              <span key={day.date} className="relative min-w-0 flex-1">
                {labelled.has(index) && (
                  <span className={cn("absolute top-0 whitespace-nowrap", index === 0 ? "left-0" : index === daily.length - 1 ? "right-0" : "left-1/2 -translate-x-1/2")}>{short(day)}</span>
                )}
              </span>
            ))}
          </div>
        </div>
      </div>

      <table className="sr-only">
        <caption>{t("stats.chartTitle", { metric: t(`stats.${metric}`) })}</caption>
        <thead>
          <tr>
            <th scope="col">{t("stats.date")}</th>
            <th scope="col">{t(`stats.${metric}`)}</th>
          </tr>
        </thead>
        <tbody>
          {daily.map((day) => (
            <tr key={day.date}>
              <th scope="row">{long(day)}</th>
              <td>{metricValue(day, metric)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}
