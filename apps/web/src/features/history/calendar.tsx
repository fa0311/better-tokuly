import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { geo } from "@tokuly/api";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { GazetteerCredit } from "#/features/places";
import { useAccountTimeZone, useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { formatDistance } from "#/lib/format";
import { historyClient, historyKeys } from "./api";
import { type Month, monthGrid, monthOfDay, parseDay, shiftMonth } from "./dates";

/** The month at a glance: active days stand out, and each leads to its diary page. */
export function HistoryCalendar() {
  const { t, i18n } = useTranslation("history");
  // The account's calendar: its "today" and its months, wherever the device is.
  const timeZone = useAccountTimeZone();
  const today = useAccountToday();
  const thisMonth = monthOfDay(today);
  const [month, setMonth] = useState<Month>(thisMonth);
  const isCurrent = month.year === thisMonth.year && month.month === thisMonth.month;

  const { data, isPending } = useQuery({
    queryKey: historyKeys.month(month, timeZone),
    queryFn: () =>
      historyClient().listActiveDays({ year: month.year, month: month.month, timeZone }),
  });
  const days = new Map((data?.days ?? []).map((day) => [day.date, day]));
  const active = [...days.values()].sort((a, b) => b.date.localeCompare(a.date));
  const title = new Date(month.year, month.month - 1, 1).toLocaleDateString(i18n.language, {
    year: "numeric",
    month: "long",
  });
  // 2023-01-01 was a Sunday: the weekday names in the person's language.
  const weekdays = Array.from({ length: 7 }, (_, index) =>
    new Date(2023, 0, 1 + index).toLocaleDateString(i18n.language, { weekday: "narrow" }),
  );

  return (
    <div className="grid gap-5 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)] lg:items-start">
      <section className="sticker rounded-card bg-surface p-4" aria-label={title}>
        <div className="flex items-center justify-between">
          <button
            type="button"
            aria-label={t("calendar.previous")}
            onClick={() => setMonth(shiftMonth(month, -1))}
            className="grid size-10 place-items-center rounded-full hover:bg-surface-2"
          >
            <ChevronLeft className="size-5" />
          </button>
          <h2 className="text-lg font-black" aria-live="polite">
            {title}
          </h2>
          <button
            type="button"
            aria-label={t("calendar.next")}
            disabled={isCurrent}
            onClick={() => setMonth(shiftMonth(month, 1))}
            className="grid size-10 place-items-center rounded-full hover:bg-surface-2 disabled:opacity-30"
          >
            <ChevronRight className="size-5" />
          </button>
        </div>
        <div
          className="mt-2 grid grid-cols-7 text-center text-xs font-black text-ink-faint"
          aria-hidden
        >
          {weekdays.map((weekday, index) => (
            <span
              key={index}
              className={cn("py-1", index === 0 && "text-pink", index === 6 && "text-sky")}
            >
              {weekday}
            </span>
          ))}
        </div>
        <div className="mt-1 grid grid-cols-7 gap-1.5" data-testid="history-calendar">
          {monthGrid(month)
            .flat()
            .map((date, index) => {
              if (!date) return <span key={index} />;
              const day = days.get(date);
              const number = Number(date.slice(8));
              const future = date > today;
              const label = parseDay(date)!.toLocaleDateString(i18n.language, {
                month: "long",
                day: "numeric",
              });
              if (!day) {
                return future ? (
                  <span
                    key={date}
                    className="grid aspect-square place-items-center rounded-xl text-sm font-bold text-ink-faint opacity-40"
                  >
                    {number}
                  </span>
                ) : (
                  <Link
                    key={date}
                    to="/history/$date"
                    params={{ date }}
                    aria-label={label}
                    className={cn(
                      "grid aspect-square place-items-center rounded-xl text-sm font-bold text-ink-soft hover:bg-surface-2",
                      date === today && "border-2 border-dashed border-ink-faint",
                    )}
                  >
                    {number}
                  </Link>
                );
              }
              return (
                <Link
                  key={date}
                  to="/history/$date"
                  params={{ date }}
                  aria-label={t("calendar.activeDay", {
                    date: label,
                    distance: formatDistance(day.distanceMeters),
                    count: day.visitCount,
                  })}
                  data-active="true"
                  className={cn(
                    "pressable sticker-sm relative grid aspect-square place-items-center overflow-hidden rounded-xl text-sm font-black",
                    day.thumbnail?.url
                      ? "bg-cover bg-center text-white"
                      : day.newCellCount > 0
                        ? "bg-lime text-[#17122b]"
                        : "bg-brand-soft text-brand-strong",
                  )}
                  style={
                    day.thumbnail?.url
                      ? {
                          backgroundImage: `linear-gradient(rgb(0 0 0 / 0.25), rgb(0 0 0 / 0.25)), url(${day.thumbnail.variants[0]?.url || day.thumbnail.url})`,
                        }
                      : undefined
                  }
                >
                  {number}
                  {day.hasMemo && (
                    <span className="absolute right-0.5 bottom-0 text-[9px]" aria-hidden>
                      📝
                    </span>
                  )}
                </Link>
              );
            })}
        </div>
        <ul className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs font-bold text-ink-soft">
          <li className="flex items-center gap-1.5">
            <span className="size-3 rounded border-2 border-ink bg-lime" aria-hidden />
            {t("calendar.legendNew")}
          </li>
          <li className="flex items-center gap-1.5">
            <span className="size-3 rounded border-2 border-ink bg-brand-soft" aria-hidden />
            {t("calendar.legendActive")}
          </li>
          <li>📝 {t("calendar.legendMemo")}</li>
        </ul>
      </section>

      <section aria-label={t("calendar.days")}>
        {isPending ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-20" />
            <Skeleton className="h-20" />
          </div>
        ) : active.length === 0 ? (
          isCurrent ? (
            <EmptyState
              emoji="📔"
              title={t("empty.title")}
              body={t("empty.body")}
              action={<MapLink label={t("empty.action")} />}
            />
          ) : (
            <EmptyState emoji="🍃" title={t("calendar.emptyMonth")} />
          )
        ) : (
          <>
            <MonthTotals days={active} />
            <ul className="mt-3 flex flex-col gap-3">
              {active.map((day) => (
                <li key={day.date}>
                  <DayRow day={day} />
                </li>
              ))}
            </ul>
            <GazetteerCredit className="mt-4 text-center" />
          </>
        )}
      </section>
    </div>
  );
}

function MapLink({ label }: { label: string }) {
  return (
    <Link
      to="/map"
      className="pressable sticker-sm inline-flex h-11 items-center rounded-full bg-lime px-5 font-black text-[#17122b]"
    >
      {label}
    </Link>
  );
}

function MonthTotals({ days }: { days: geo.DaySummary[] }) {
  const { t } = useTranslation("history");
  const distance = days.reduce((sum, day) => sum + day.distanceMeters, 0);
  const visits = days.reduce((sum, day) => sum + day.visitCount, 0);
  const cells = days.reduce((sum, day) => sum + day.newCellCount, 0);
  return (
    <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
      {t("calendar.totals", { days: days.length, distance: formatDistance(distance), visits })}
      {cells > 0 && ` ${t("calendar.totalsCells", { count: cells })}`}
    </p>
  );
}

function DayRow({ day }: { day: geo.DaySummary }) {
  const { t, i18n } = useTranslation("history");
  const date = parseDay(day.date);
  return (
    <Link
      to="/history/$date"
      params={{ date: day.date }}
      className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
    >
      <span className="grid size-14 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-center leading-none text-[#17122b]">
        <span>
          <span className="block text-xl font-black">{date?.getDate()}</span>
          <span className="block text-[10px] font-black">
            {date?.toLocaleDateString(i18n.language, { weekday: "short" })}
          </span>
        </span>
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">
          {day.topLocality || t("calendar.somewhere")}
        </span>
        <span className="block truncate text-sm text-ink-soft">
          {formatDistance(day.distanceMeters)}・{t("day.visits", { count: day.visitCount })}
          {day.newCellCount > 0 && `・${t("day.newCells", { count: day.newCellCount })}`}
        </span>
      </span>
      {day.thumbnail?.url ? (
        <img
          src={day.thumbnail.variants[0]?.url || day.thumbnail.url}
          alt=""
          className="size-12 shrink-0 rounded-xl object-cover"
        />
      ) : (
        day.hasMemo && (
          <span className="shrink-0 text-xl" title={t("calendar.legendMemo")}>
            📝
          </span>
        )
      )}
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </Link>
  );
}
