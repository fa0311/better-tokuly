import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { school } from "@tokuly/api";
import {
  addDays,
  dayLabel,
  mondayOf,
  parseYmd,
  schoolKeys,
  studyTime,
  takesPlace,
  timerText,
  weekdayOf,
} from "@tokuly/core/school";
import { ChevronLeft, ChevronRight, Timer } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Skeleton } from "#/components/ui/page";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { timetableClient, useProgress } from "./api";

import { DayView } from "./day-view";
import { DueList } from "./due";
import { ClassNowCard } from "./now-card";
import { Block, SchoolPage, useNow } from "./parts";
import { useActiveSession } from "./session-store";
import { StudyingFriends } from "./study-social";
import { studiedSecondsAt } from "./timer";

/**
 * The front page of the school area: the class that is on and the next one,
 * the day's classes (any day of any week, with one-off changes), what is
 * due, and how today's studying is going.
 */
export function TodayScreen() {
  const { t, i18n } = useTranslation("school");
  const today = useAccountToday();
  // No day picked means "today", so a page left open overnight moves on with the date.
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const date = picked ?? today;
  const setDate = (next: string) => setPicked(next === today ? undefined : next);
  const monday = mondayOf(date);
  const now = useNow(30_000);
  const week = useQuery({
    queryKey: schoolKeys.days(monday, addDays(monday, 6)),
    queryFn: async () =>
      (await timetableClient().listDays({ startDate: monday, endDate: addDays(monday, 6) })).days,
  });
  const day = useQuery({
    queryKey: schoolKeys.day(date),
    queryFn: async () => (await timetableClient().getDay({ date })).day,
  });

  return (
    <SchoolPage section="today" wide>
      <div className="grid gap-x-6 lg:grid-cols-2 lg:items-start">
        <div className="min-w-0">
          <ClassNowCard />
          <Block
            title={
              date === today
                ? t("today.classesToday")
                : t("today.classesOn", { date: dayLabel(date, i18n.language) })
            }
            action={
              date !== today && (
                <Button size="sm" variant="ghost" onClick={() => setDate(today)}>
                  {t("today.backToToday")}
                </Button>
              )
            }
          >
            <div className="mb-3 flex items-center gap-1">
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("today.previousWeek")}
                onClick={() => setDate(addDays(date, -7))}
              >
                <ChevronLeft className="size-5" />
              </Button>
              <ol className="grid min-w-0 flex-1 grid-cols-7 gap-1" aria-label={t("today.week")}>
                {Array.from({ length: 7 }, (_, offset) => {
                  const value = addDays(monday, offset);
                  const info = week.data?.find((item) => item.date === value);
                  const count = info?.classes.filter(takesPlace).length ?? 0;
                  const off = info !== undefined && info.kind !== school.DayKind.SCHOOL_DAY;
                  return (
                    <li key={value}>
                      <button
                        type="button"
                        aria-pressed={value === date}
                        aria-label={`${dayLabel(value, i18n.language)} ${info ? t("today.classCount", { count }) : ""}`}
                        onClick={() => setDate(value)}
                        className={cn(
                          "flex w-full flex-col items-center rounded-2xl py-1.5 leading-tight transition-colors",
                          value === date ? "sticker-sm bg-ink text-surface" : "hover:bg-surface-2",
                          value === today && value !== date && "bg-brand-soft",
                        )}
                      >
                        <span className="text-[11px] font-bold opacity-70">
                          {t(`weekdays.short.${weekdayOf(value)}`)}
                        </span>
                        <span className="font-black">{parseYmd(value).getDate()}</span>
                        <span
                          className={cn("text-[10px] font-bold", off ? "text-coral" : "opacity-70")}
                        >
                          {info ? (off ? t("today.off") : count) : " "}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ol>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("today.nextWeek")}
                onClick={() => setDate(addDays(date, 7))}
              >
                <ChevronRight className="size-5" />
              </Button>
            </div>
            {day.isPending ? (
              <Skeleton className="h-64" />
            ) : day.data ? (
              <DayView day={day.data} editable nowMs={date === today ? now : undefined} />
            ) : null}
            {day.data?.kind === school.DayKind.OUT_OF_TERM && (
              <Link
                to="/school/timetable"
                className="mt-3 block text-center text-sm font-bold text-brand"
              >
                {t("today.setUpTimetable")}
              </Link>
            )}
            {day.data?.kind === school.DayKind.SCHOOL_DAY && (
              <p className="mt-2 px-1 text-xs text-ink-faint">{t("today.changeHint")}</p>
            )}
          </Block>
        </div>
        <div className="min-w-0">
          <Block title={t("due.title")} className="lg:mt-0" action={<SeeAll to="/school/exams" />}>
            <DueList limit={5} />
          </Block>
          <Block title={t("today.study")} action={<SeeAll to="/school/study" />}>
            <StudyStrip />
          </Block>
          <Block title={t("study.friendsTitle")}>
            <StudyingFriends />
          </Block>
        </div>
      </div>
    </SchoolPage>
  );
}

function SeeAll({ to }: { to: "/school/exams" | "/school/study" }) {
  const { t } = useTranslation("school");
  return (
    <Link to={to} className="flex items-center text-sm font-bold text-brand">
      {t("today.seeAll")}
      <ChevronRight className="size-4" />
    </Link>
  );
}

/** Today's study time and streak in one line, with the way to the timer (or the timer itself while it runs). */
function StudyStrip() {
  const { t, i18n } = useTranslation("school");
  const progress = useProgress();
  const session = useActiveSession((state) => state.session);
  const now = useNow(1000);
  return (
    <Link
      to="/school/study"
      className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4"
    >
      <span
        className={cn(
          "grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink",
          session ? "bg-brand text-on-brand" : "bg-lime text-[#17122b]",
        )}
      >
        <Timer className="size-6" />
      </span>
      <span className="min-w-0 flex-1">
        {session ? (
          <>
            <span className="block text-xs font-black text-brand">
              {session.state === school.SessionState.PAUSED
                ? t("timer.phase.paused")
                : t("study.studying")}
            </span>
            <span className="block text-2xl font-black tabular-nums leading-tight">
              {timerText(studiedSecondsAt(session, now))}
            </span>
          </>
        ) : (
          <>
            <span className="block text-xs font-black text-ink-soft">{t("study.today")}</span>
            <span className="block text-2xl font-black leading-tight">
              {studyTime(progress.data?.todaySeconds ?? 0, i18n.language)}
            </span>
          </>
        )}
      </span>
      <span className="shrink-0 text-right text-sm font-bold">
        <span className="block">
          🔥 {t("study.streakDays", { count: progress.data?.streakDays ?? 0 })}
        </span>
        {!session && <span className="block text-brand">{t("today.startTimer")}</span>}
      </span>
    </Link>
  );
}
