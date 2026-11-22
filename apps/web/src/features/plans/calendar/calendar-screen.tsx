import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { plans, toDate } from "@tokuly/api";
import { CalendarPlus, ChevronRight, Download, Hourglass } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { Page, Skeleton } from "#/components/ui/page";
import { useOpenTargetPath } from "#/features/notifications";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { keys, planClient, useCalendar } from "../api";
import { CountdownDialog } from "../countdown/countdown-dialog";
import { localized } from "../countdown/model";
import { formatDay, formatMonth, formatTime, localTimeZone, monthOf, todayYmd } from "../dates";
import { setPlanDraft } from "../draft";
import { buildIcs, downloadIcs } from "../ics";
import { RsvpChip } from "../plan/plan-card";
import { calendarIcsEvents, entryTitle } from "./ics-events";
import { MonthGrid } from "./month-grid";

const KIND: Record<number, { emoji: string; dot: string; key: "plan" | "poll" | "countdown" | "anniversary" | "birthday" }> = {
  [plans.CalendarEntryKind.PLAN]: { emoji: "📅", dot: "bg-brand", key: "plan" },
  [plans.CalendarEntryKind.POLL_DEADLINE]: { emoji: "🗳️", dot: "bg-sky", key: "poll" },
  [plans.CalendarEntryKind.COUNTDOWN]: { emoji: "⏳", dot: "bg-sun", key: "countdown" },
  [plans.CalendarEntryKind.ANNIVERSARY]: { emoji: "💞", dot: "bg-pink", key: "anniversary" },
  [plans.CalendarEntryKind.BIRTHDAY]: { emoji: "🎂", dot: "bg-coral", key: "birthday" },
};
const kindOf = (entry: plans.CalendarEntry) => KIND[entry.kind] ?? KIND[plans.CalendarEntryKind.PLAN]!;

/**
 * `/plans/calendar`: the month with everything dated — plans, poll
 * deadlines, countdowns, milestones, friends' birthdays — the day that is
 * tapped spelled out next to it, and the month as an `.ics` file.
 */
export function CalendarScreen({ month, day, onChange }: { month: string; day: string; onChange(next: { month: string; day: string }): void }) {
  const { t, i18n } = useTranslation("plans");
  const navigate = useNavigate();
  const openPath = useOpenTargetPath();
  const queryClient = useQueryClient();
  const calendar = useCalendar(month);
  const [counting, setCounting] = useState(false);
  const today = todayYmd();
  const entries = calendar.data ?? [];
  const byDay = new Map<string, plans.CalendarEntry[]>();
  for (const entry of entries) byDay.set(entry.date, [...(byDay.get(entry.date) ?? []), entry]);
  const dayEntries = byDay.get(day) ?? [];

  const exportMonth = useMutation({
    mutationFn: async () => {
      // A calendar entry only knows when a plan starts; the plan itself has its end, place and notes.
      const planIds = [...new Set(entries.filter((entry) => entry.kind === plans.CalendarEntryKind.PLAN && entry.refId).map((entry) => entry.refId))];
      const loaded = await Promise.all(
        planIds.map((planId) =>
          queryClient
            .fetchQuery({ queryKey: keys.plan(planId), queryFn: async () => (await planClient().getPlan({ planId })).plan ?? null, staleTime: 30_000 })
            // A plan that cannot be read is written from its entry.
            .catch(() => null),
        ),
      );
      const plansById = new Map(loaded.flatMap((plan) => (plan ? [[plan.id, plan] as const] : [])));
      downloadIcs(`tokuly-${month}.ics`, buildIcs(calendarIcsEvents(entries, plansById, i18n.language, window.location.origin)));
    },
    onSuccess: () => toast.success(t("calendar.exported", { count: entries.length })),
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });
  const planOnDay = () => {
    setPlanDraft({ time: { start: { case: "startDate", value: day }, end: { case: undefined }, timeZone: localTimeZone() } as plans.PlanTime });
    void navigate({ to: "/plans/new" });
  };

  return (
    <Page
      title={t("calendar.title")}
      back="/plans"
      wide
      actions={
        <>
          {(month !== monthOf(today) || day !== today) && (
            <Button variant="secondary" size="sm" onClick={() => onChange({ month: monthOf(today), day: today })}>
              {t("calendar.today")}
            </Button>
          )}
          <Button variant="ghost" size="icon" aria-label={t("calendar.export")} loading={exportMonth.isPending} onClick={() => (entries.length === 0 ? toast(t("calendar.exportEmpty")) : exportMonth.mutate())}>
            {!exportMonth.isPending && <Download className="size-5" />}
          </Button>
        </>
      }
    >
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,6fr)_minmax(0,5fr)]">
        <Card className="p-3 sm:p-5">
          <MonthGrid
            month={month}
            onMonthChange={(next) => onChange({ month: next, day: next === monthOf(today) ? today : `${next}-01` })}
            onPick={(picked) => onChange({ month: monthOf(picked), day: picked })}
            isPressed={(cellDay) => cellDay === day}
            dayLabel={(cell) => {
              const count = byDay.get(cell.day)?.length ?? 0;
              return count > 0 ? t("calendar.dayLabel", { day: formatDay(cell.day, i18n.language), count }) : formatDay(cell.day, i18n.language);
            }}
            cellClassName={(cell) => (cell.day === day ? "border-ink bg-brand-soft shadow-pop-sm" : "hover:bg-surface-2")}
            renderCell={(cell) => {
              const items = cell.inMonth ? (byDay.get(cell.day) ?? []) : [];
              if (items.length === 0) return null;
              return (
                <span className="flex max-w-full flex-wrap items-center justify-center gap-0.5" aria-hidden data-testid="calendar-marks">
                  {/* Wide cells show what is on; narrow ones a dot per kind. */}
                  <span className="hidden text-base leading-none sm:inline">{items[0]!.emoji || kindOf(items[0]!).emoji}</span>
                  {items.length > 1 && <span className="hidden text-[10px] font-black text-ink-soft sm:inline">+{items.length - 1}</span>}
                  {items.slice(0, 3).map((entry, index) => (
                    <span key={index} className={cn("size-1.5 rounded-full sm:hidden", kindOf(entry).dot)} />
                  ))}
                </span>
              );
            }}
          />
          <ul className="mt-3 flex flex-wrap gap-x-3 gap-y-1 px-1 text-xs font-bold text-ink-soft" aria-label={t("calendar.legend")}>
            {Object.values(KIND).map((kind) => (
              <li key={kind.key} className="flex items-center gap-1">
                <span className={cn("size-2 rounded-full", kind.dot)} aria-hidden />
                {t(`calendar.kind.${kind.key}`)}
              </li>
            ))}
          </ul>
        </Card>

        <section aria-label={formatDay(day, i18n.language)} data-testid="calendar-day">
          <div className="mb-2 flex items-center gap-2 px-1">
            <h2 className="min-w-0 flex-1 truncate text-lg font-black">
              {formatDay(day, i18n.language)}
              {day === today && <span className="ml-2 rounded-full bg-lime px-2 py-0.5 text-xs text-[#17122b]">{t("plan.today")}</span>}
            </h2>
          </div>
          {calendar.isPending ? (
            <Skeleton className="h-24 rounded-card" />
          ) : dayEntries.length === 0 ? (
            <p className="rounded-card border-2 border-dashed border-line px-4 py-8 text-center text-sm font-bold text-ink-soft">{t("calendar.emptyDay")}</p>
          ) : (
            <ul className="flex flex-col gap-2">
              {dayEntries.map((entry, index) => {
                const start = toDate(entry.startAt);
                const label = entry.title ? localized(entry.label, i18n.language) : "";
                return (
                  <li key={`${entry.kind}-${entry.refId}-${index}`}>
                    <button type="button" disabled={!entry.path} onClick={() => openPath(entry.path)} className="pressable sticker flex w-full items-center gap-3 rounded-card bg-surface p-3 text-left" data-testid="calendar-entry">
                      <span className="grid size-12 shrink-0 place-items-center rounded-2xl bg-surface-2 text-2xl" aria-hidden>
                        {entry.emoji || kindOf(entry).emoji}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-black">{entryTitle(entry, i18n.language)}</span>
                        <span className="flex flex-wrap items-center gap-x-2 text-sm font-bold text-ink-soft">
                          <span className="flex items-center gap-1">
                            <span className={cn("size-2 rounded-full", kindOf(entry).dot)} aria-hidden />
                            {label || t(`calendar.kind.${kindOf(entry).key}`)}
                          </span>
                          <span>{start ? formatTime(start, i18n.language) : t("calendar.allDay")}</span>
                        </span>
                      </span>
                      {entry.kind === plans.CalendarEntryKind.PLAN && entry.rsvp !== plans.Rsvp.UNSPECIFIED && <RsvpChip rsvp={entry.rsvp} className="shrink-0" />}
                      {entry.user && entry.kind !== plans.CalendarEntryKind.PLAN && <Avatar user={entry.user} size="sm" />}
                      {entry.path && <ChevronRight className="size-4 shrink-0 text-ink-faint" />}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
          {day >= today && (
            <div className="mt-3 grid grid-cols-2 gap-2.5">
              <Button icon={<CalendarPlus className="size-4" />} onClick={planOnDay}>
                {t("calendar.planOnDay")}
              </Button>
              <Button variant="secondary" icon={<Hourglass className="size-4" />} onClick={() => setCounting(true)}>
                {t("calendar.countdownOnDay")}
              </Button>
            </div>
          )}
          <p className="mt-4 px-1 text-xs text-ink-faint">{t("calendar.monthSummary", { month: formatMonth(month, i18n.language), count: entries.length })}</p>
        </section>
      </div>
      <CountdownDialog open={counting} onOpenChange={setCounting} initialDate={day} />
    </Page>
  );
}
