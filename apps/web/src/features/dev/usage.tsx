import { useQuery } from "@tanstack/react-query";
import { errorReason } from "@tokuly/api";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { type dev, keys, type UsageSubject, usageClient } from "./api";
import { Notice } from "./parts";

const DAYS = 30;

/**
 * The days of the chart, oldest first, with zero for days without traffic.
 * The API counts by UTC day, so this is the UTC calendar on purpose (it is a
 * chart of the API's days, not of the account's "today").
 */
export function fillDays(days: dev.UsageDay[], count: number, nowMs = Date.now()) {
  const byDate = new Map(days.map((day) => [day.date, day]));
  return Array.from({ length: count }, (_, index) => {
    const date = new Date(nowMs - (count - 1 - index) * 86_400_000).toISOString().slice(0, 10);
    const day = byDate.get(date);
    return { date, requests: Number(day?.requests ?? 0), errors: Number(day?.errors ?? 0) };
  });
}

/** Requests and errors of one token, application or bot over the last 30 days. */
export function UsagePanel({ subject }: { subject: UsageSubject }) {
  const { t } = useTranslation("dev");
  const usage = useQuery({
    queryKey: keys.usage(subject.kind, subject.id, DAYS),
    queryFn: () =>
      usageClient().getUsage({
        subject: { case: `${subject.kind}Id`, value: subject.id },
        days: DAYS,
      }),
  });

  if (usage.isPending) return <Skeleton className="h-40" />;
  if (usage.isError)
    return (
      <Notice tone={errorReason(usage.error) === "ANALYTICS_UNAVAILABLE" ? "info" : "danger"}>
        {errorMessage(usage.error, { namespace: "dev" })}
      </Notice>
    );

  const total = Number(usage.data.requests);
  const errors = Number(usage.data.errors);
  if (total === 0) return <Notice>{t("usage.empty")}</Notice>;

  const days = fillDays(usage.data.days, DAYS);
  const peak = Math.max(1, ...days.map((day) => day.requests));
  return (
    <div className="flex flex-col gap-4">
      <dl className="grid grid-cols-3 gap-2 text-center">
        <Stat label={t("usage.requests")} value={total.toLocaleString()} />
        <Stat label={t("usage.errors")} value={errors.toLocaleString()} />
        <Stat label={t("usage.errorRate")} value={`${((errors / total) * 100).toFixed(1)}%`} />
      </dl>

      <figure>
        <figcaption className="mb-1.5 px-1 text-xs font-bold text-ink-soft">
          {t("usage.byDay", { count: DAYS })}
        </figcaption>
        <ol className="flex h-24 items-end gap-px rounded-2xl bg-surface-2 p-2">
          {days.map((day) => (
            <li
              key={day.date}
              title={t("usage.dayTitle", day)}
              className="flex h-full min-w-0 flex-1 flex-col justify-end"
            >
              {/* Errors sit on top of the day's bar, in the danger colour. */}
              <span
                className="rounded-t-sm bg-danger"
                style={{ height: `${(day.errors / peak) * 100}%` }}
              />
              <span
                className={cn("bg-brand", day.errors === 0 && "rounded-t-sm")}
                style={{
                  height: `${((day.requests - day.errors) / peak) * 100}%`,
                  minHeight: day.requests > 0 ? 2 : 0,
                }}
              />
            </li>
          ))}
        </ol>
        <p className="mt-1 flex justify-between px-1 font-mono text-[11px] text-ink-faint">
          <span>{days[0]?.date}</span>
          <span>{days.at(-1)?.date} UTC</span>
        </p>
      </figure>

      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead className="text-xs text-ink-soft">
            <tr>
              <th className="py-1.5 pr-3 font-bold">{t("usage.method")}</th>
              <th className="px-2 py-1.5 text-right font-bold">{t("usage.requests")}</th>
              <th className="px-2 py-1.5 text-right font-bold">{t("usage.errors")}</th>
              <th className="py-1.5 pl-2 text-right font-bold">{t("usage.average")}</th>
            </tr>
          </thead>
          <tbody className="divide-y-2 divide-line">
            {usage.data.methods.map((method) => (
              <tr key={method.procedure}>
                <td className="py-1.5 pr-3 font-mono text-xs break-all">{method.procedure}</td>
                <td className="px-2 py-1.5 text-right tabular-nums">
                  {Number(method.requests).toLocaleString()}
                </td>
                <td
                  className={cn(
                    "px-2 py-1.5 text-right tabular-nums",
                    method.errors > 0n && "font-bold text-danger",
                  )}
                >
                  {Number(method.errors).toLocaleString()}
                </td>
                <td className="py-1.5 pl-2 text-right tabular-nums">{method.averageMs} ms</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl bg-surface-2 px-2 py-2.5">
      <dt className="text-xs font-bold text-ink-soft">{label}</dt>
      <dd className="text-xl font-black tabular-nums">{value}</dd>
    </div>
  );
}
