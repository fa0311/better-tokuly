import { useQuery } from "@tanstack/react-query";
import {
  addDays,
  dayLabel,
  daysBetween,
  parseYmd,
  schoolKeys,
  studyTime,
  weekdayOf,
} from "@tokuly/core/school";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { studyClient } from "./api";

const LEVELS = ["bg-surface-3", "bg-brand/25", "bg-brand/50", "bg-brand/75", "bg-brand"];

/**
 * A year of study as a calendar of small squares, one column per week
 * (Monday on top), darker for longer days. Someone else's when `userId` is set.
 */
export function Heatmap({ userId = "" }: { userId?: string }) {
  const { t, i18n } = useTranslation("school");
  const { data, isPending, isError } = useQuery({
    queryKey: schoolKeys.heatmap(userId),
    queryFn: () => studyClient().getStudyHeatmap({ userId }),
  });
  if (isPending) return <Skeleton className="h-36" />;
  if (isError || !data)
    return (
      <p className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft">
        {t("heatmap.notShared")}
      </p>
    );

  const totals = new Map(data.days.map((day) => [day.date, day.seconds]));
  const first = addDays(data.startDate, 1 - weekdayOf(data.startDate));
  const weeks = Math.ceil((daysBetween(first, data.endDate) + 1) / 7);
  const level = (seconds: number) =>
    seconds <= 0
      ? 0
      : Math.min(4, 1 + Math.floor((seconds / Math.max(1, data.maxSeconds)) * 3.999));

  return (
    <div className="rounded-card bg-surface p-4">
      <p className="mb-3 text-sm font-bold text-ink-soft">
        {t("heatmap.total", {
          time: studyTime(data.totalSeconds, i18n.language),
          days: data.days.length,
        })}
      </p>
      {/* Newest weeks first in the scroll: the list is laid out right to left. */}
      <div className="scrollbar-none flex flex-row-reverse overflow-x-auto" dir="ltr">
        <ol className="flex gap-[3px]" aria-label={t("heatmap.title")}>
          {Array.from({ length: weeks }, (_, week) => {
            const monday = addDays(first, week * 7);
            const month =
              parseYmd(monday).getDate() <= 7 ? parseYmd(monday).getMonth() + 1 : undefined;
            return (
              <li key={monday} className="flex flex-col gap-[3px]">
                <span aria-hidden className="h-3 text-[9px] font-bold leading-3 text-ink-faint">
                  {month}
                </span>
                {Array.from({ length: 7 }, (_, offset) => {
                  const date = addDays(monday, offset);
                  if (date < data.startDate || date > data.endDate)
                    return <span key={date} className="size-3.5" />;
                  const seconds = totals.get(date) ?? 0;
                  const label = `${dayLabel(date, i18n.language)} ${studyTime(seconds, i18n.language)}`;
                  return (
                    <span
                      key={date}
                      role="img"
                      title={label}
                      aria-label={label}
                      data-level={level(seconds)}
                      className={cn("size-3.5 rounded-[4px]", LEVELS[level(seconds)])}
                    />
                  );
                })}
              </li>
            );
          })}
        </ol>
      </div>
      <p className="mt-3 flex items-center justify-end gap-1 text-[11px] font-bold text-ink-faint">
        {t("heatmap.less")}
        {LEVELS.map((className) => (
          <span key={className} className={cn("size-3 rounded-[3px]", className)} />
        ))}
        {t("heatmap.more")}
      </p>
    </div>
  );
}
