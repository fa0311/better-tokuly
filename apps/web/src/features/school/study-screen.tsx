import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { school } from "@tokuly/api";
import { Flame, Target } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { keys, studyClient, useProgress } from "./api";
import { addDays, dayLabel, daysBetween, parseYmd, studyTime, weekdayOf, ymd } from "./dates";
import { Block, Figure, SchoolPage, SubjectPill } from "./parts";
import { StudyLog } from "./study-log";
import { Ranking, StudyingFriends } from "./study-social";
import { TimerCard } from "./timer-card";

/** The study page: the timer, today against the goal, totals, the heatmap, friends and the log. */
export function StudyScreen() {
  const { t } = useTranslation("school");
  return (
    <SchoolPage section="study" wide>
      <div className="grid gap-x-6 lg:grid-cols-[minmax(0,24rem)_minmax(0,1fr)] lg:items-start">
        <div className="min-w-0">
          <TimerCard />
          <ProgressCard />
          <Block title={t("study.friendsTitle")}>
            <StudyingFriends />
          </Block>
          <Block title={t("ranking.title")}>
            <Ranking />
          </Block>
        </div>
        <div className="min-w-0">
          <Block title={t("study.totals")} className="lg:mt-0">
            <Summary />
          </Block>
          <Block title={t("heatmap.title")}>
            <Heatmap />
          </Block>
          <StudyLog />
        </div>
      </div>
    </SchoolPage>
  );
}

/** Today against the daily goal, the streak, the week and everything so far. */
export function ProgressCard({ className }: { className?: string }) {
  const { t, i18n } = useTranslation("school");
  const { data, isPending } = useProgress();
  if (isPending) return <Skeleton className={cn("mt-4 h-32", className)} />;
  if (!data) return null;
  const goalSeconds = data.goalMinutes * 60;
  const fraction = goalSeconds > 0 ? Math.min(1, data.todaySeconds / goalSeconds) : 0;
  return (
    <section aria-label={t("study.progress")} className={cn("mt-4 rounded-card bg-surface p-4", className)}>
      <div className="flex items-end gap-4">
        <Figure className="flex-1" value={studyTime(data.todaySeconds, i18n.language)} label={t("study.today")} />
        <Figure
          value={
            <span className="inline-flex items-center gap-1">
              <Flame className={cn("size-5", data.streakDays > 0 ? "text-coral" : "text-ink-faint")} />
              {t("study.streakDays", { count: data.streakDays })}
            </span>
          }
          label={t("study.streak", { count: data.longestStreakDays })}
        />
      </div>
      {goalSeconds > 0 ? (
        <div className="mt-3">
          <div role="progressbar" aria-label={t("study.goal")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)} className="h-4 overflow-hidden rounded-full border-2 border-ink bg-surface-3">
            <div className={cn("h-full transition-[width] duration-500", data.goalReached ? "bg-lime" : "bg-brand")} style={{ width: `${fraction * 100}%` }} />
          </div>
          <p className="mt-1.5 flex items-center gap-1 text-xs font-bold text-ink-soft">
            <Target className="size-3.5" />
            {data.goalReached ? t("study.goalReached", { goal: studyTime(goalSeconds, i18n.language) }) : t("study.goalLeft", { goal: studyTime(goalSeconds, i18n.language), left: studyTime(goalSeconds - data.todaySeconds, i18n.language) })}
          </p>
        </div>
      ) : (
        <Link to="/school/settings" className="mt-3 flex items-center gap-1 text-sm font-bold text-brand">
          <Target className="size-4" />
          {t("study.setGoal")}
        </Link>
      )}
      <div className="mt-3 grid grid-cols-2 gap-3 border-t-2 border-line pt-3">
        <Figure value={studyTime(data.weekSeconds, i18n.language)} label={t("study.thisWeek")} />
        <Figure value={studyTime(Number(data.totalSeconds), i18n.language)} label={t("study.allTime")} />
      </div>
    </section>
  );
}

type PeriodTab = "day" | "week" | "month";
const PERIODS: Record<PeriodTab, school.StatsPeriod> = { day: school.StatsPeriod.DAY, week: school.StatsPeriod.WEEK, month: school.StatsPeriod.MONTH };

/** Study time of today, this week or this month: the total, each day as a bar, each subject's share. */
function Summary() {
  const { t, i18n } = useTranslation("school");
  const [tab, setTab] = useState<PeriodTab>("week");
  const { data, isPending } = useQuery({ queryKey: keys.summary(PERIODS[tab]), queryFn: () => studyClient().getStudySummary({ period: PERIODS[tab] }) });
  const max = Math.max(1, ...(data?.days ?? []).map((day) => day.seconds));
  const difference = data ? data.totalSeconds - data.previousTotalSeconds : 0;
  const today = ymd();

  return (
    <div className="rounded-card bg-surface p-4">
      <Tabs
        value={tab}
        onChange={(next: PeriodTab) => setTab(next)}
        items={[
          { value: "day", label: t("study.period.day") },
          { value: "week", label: t("study.period.week") },
          { value: "month", label: t("study.period.month") },
        ]}
      />
      {isPending || !data ? (
        <Skeleton className="mt-4 h-40" />
      ) : (
        <>
          <div className="mt-4 flex items-end gap-3">
            <Figure className="flex-1" value={studyTime(data.totalSeconds, i18n.language)} label={t(`study.periodTotal.${tab}`)} />
            {data.previousTotalSeconds > 0 || data.totalSeconds > 0 ? (
              <p className={cn("rounded-full px-2.5 py-1 text-xs font-black", difference >= 0 ? "bg-lime text-[#17122b]" : "bg-surface-3 text-ink-soft")}>
                {t(`study.versus.${tab}`)} {difference >= 0 ? "+" : "−"}
                {studyTime(Math.abs(difference), i18n.language)}
              </p>
            ) : null}
          </div>
          {data.days.length > 1 && (
            <ol className="mt-4 flex h-28 items-end gap-1" aria-label={t("study.byDay")}>
              {data.days.map((day) => (
                <li key={day.date} className="flex h-full min-w-0 flex-1 flex-col items-center justify-end gap-1" title={`${dayLabel(day.date, i18n.language)} ${studyTime(day.seconds, i18n.language)}`}>
                  <span className="sr-only">
                    {dayLabel(day.date, i18n.language)} {studyTime(day.seconds, i18n.language)}
                  </span>
                  <span aria-hidden className={cn("w-full max-w-8 rounded-t-md", day.seconds > 0 ? "border-2 border-b-0 border-ink bg-brand" : "h-0.5 bg-surface-3", day.date === today && day.seconds > 0 && "bg-lime")} style={day.seconds > 0 ? { height: `${Math.max(6, (day.seconds / max) * 100)}%` } : undefined} />
                  {data.days.length <= 7 && <span aria-hidden className={cn("text-[10px] font-black", day.date === today ? "text-ink" : "text-ink-faint")}>{t(`weekdays.short.${weekdayOf(day.date)}`)}</span>}
                </li>
              ))}
            </ol>
          )}
          {data.subjects.length > 0 ? (
            <ul className="mt-4 flex flex-col gap-2" aria-label={t("study.bySubject")}>
              {data.subjects.map((item) => (
                <li key={item.subject?.id ?? "none"} className="flex items-center gap-2">
                  <SubjectPill subject={item.subject} className="shrink-0" />
                  <span className="h-2.5 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-3">
                    <span className="block h-full rounded-full" style={{ width: `${(item.seconds / Math.max(1, data.totalSeconds)) * 100}%`, background: item.subject?.color ?? "var(--ink-faint)" }} />
                  </span>
                  <span className="w-20 shrink-0 text-right text-sm font-bold">{studyTime(item.seconds, i18n.language)}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-4 text-sm text-ink-soft">{t("study.noTime")}</p>
          )}
        </>
      )}
    </div>
  );
}

const LEVELS = ["bg-surface-3", "bg-brand/25", "bg-brand/50", "bg-brand/75", "bg-brand"];

/**
 * A year of study as a calendar of small squares, one column per week
 * (Monday on top), darker for longer days. Someone else's when `userId` is set.
 */
export function Heatmap({ userId = "" }: { userId?: string }) {
  const { t, i18n } = useTranslation("school");
  const { data, isPending, isError } = useQuery({ queryKey: keys.heatmap(userId), queryFn: () => studyClient().getStudyHeatmap({ userId }) });
  if (isPending) return <Skeleton className="h-36" />;
  if (isError || !data) return <p className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft">{t("heatmap.notShared")}</p>;

  const totals = new Map(data.days.map((day) => [day.date, day.seconds]));
  const first = addDays(data.startDate, 1 - weekdayOf(data.startDate));
  const weeks = Math.ceil((daysBetween(first, data.endDate) + 1) / 7);
  const level = (seconds: number) => (seconds <= 0 ? 0 : Math.min(4, 1 + Math.floor((seconds / Math.max(1, data.maxSeconds)) * 3.999)));

  return (
    <div className="rounded-card bg-surface p-4">
      <p className="mb-3 text-sm font-bold text-ink-soft">{t("heatmap.total", { time: studyTime(data.totalSeconds, i18n.language), days: data.days.length })}</p>
      {/* Newest weeks first in the scroll: the list is laid out right to left. */}
      <div className="scrollbar-none flex flex-row-reverse overflow-x-auto" dir="ltr">
        <ol className="flex gap-[3px]" aria-label={t("heatmap.title")}>
          {Array.from({ length: weeks }, (_, week) => {
            const monday = addDays(first, week * 7);
            const month = parseYmd(monday).getDate() <= 7 ? parseYmd(monday).getMonth() + 1 : undefined;
            return (
              <li key={monday} className="flex flex-col gap-[3px]">
                <span aria-hidden className="h-3 text-[9px] font-bold leading-3 text-ink-faint">
                  {month}
                </span>
                {Array.from({ length: 7 }, (_, offset) => {
                  const date = addDays(monday, offset);
                  if (date < data.startDate || date > data.endDate) return <span key={date} className="size-3.5" />;
                  const seconds = totals.get(date) ?? 0;
                  const label = `${dayLabel(date, i18n.language)} ${studyTime(seconds, i18n.language)}`;
                  return <span key={date} role="img" title={label} aria-label={label} data-level={level(seconds)} className={cn("size-3.5 rounded-[4px]", LEVELS[level(seconds)])} />;
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
