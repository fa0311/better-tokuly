import { Link } from "@tanstack/react-router";
import { latenessText } from "@tokuly/core/moments";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Page, Skeleton } from "#/components/ui/page";
import { imageUrl } from "#/features/notes";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useMomentCalendar } from "./api";

const pad = (value: number) => String(value).padStart(2, "0");

/** The caller's own moments, a month at a time: only they ever see past days. */
export function MomentCalendarScreen() {
  const { t, i18n } = useTranslation("moments");
  const today = useAccountToday();
  const [year, monthNow] = today.split("-").map(Number) as [number, number];
  const [month, setMonth] = useState({ year, month: monthNow });
  const calendar = useMomentCalendar(month.year, month.month);
  const days = new Map(calendar.data?.days.map((day) => [day.date, day]));
  const first = new Date(month.year, month.month - 1, 1);
  const dayCount = new Date(month.year, month.month, 0).getDate();
  const isCurrent = month.year === year && month.month === monthNow;
  const move = (delta: number) => {
    const index = month.year * 12 + (month.month - 1) + delta;
    setMonth({ year: Math.floor(index / 12), month: (index % 12) + 1 });
  };
  const weekdays = Array.from({ length: 7 }, (_, index) =>
    new Date(2023, 0, 1 + index).toLocaleDateString(i18n.language, { weekday: "narrow" }),
  );

  return (
    <Page title={t("calendar.title")} back={{ fallback: "/moments" }}>
      <p className="mb-3 text-sm font-bold text-ink-soft">{t("calendar.lead")}</p>
      <section className="sticker rounded-card bg-surface p-4" data-testid="moment-calendar">
        <div className="flex items-center gap-2">
          <h2 className="flex-1 text-lg font-black">
            {first.toLocaleDateString(i18n.language, { year: "numeric", month: "long" })}
          </h2>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("calendar.previous")}
            onClick={() => move(-1)}
          >
            <ChevronLeft className="size-5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("calendar.next")}
            disabled={isCurrent}
            onClick={() => move(1)}
          >
            <ChevronRight className="size-5" />
          </Button>
        </div>
        {calendar.data && (
          <p
            className="mt-1 text-xs font-black text-ink-soft"
            data-testid="moment-calendar-summary"
          >
            {t("calendar.summary", {
              posted: calendar.data.postedCount,
              onTime: calendar.data.onTimeCount,
            })}
          </p>
        )}
        <div className="mt-3 grid grid-cols-7 gap-1 text-center">
          {weekdays.map((weekday, index) => (
            <span key={index} className="pb-1 text-xs font-black text-ink-faint">
              {weekday}
            </span>
          ))}
          {Array.from({ length: first.getDay() }, (_, index) => (
            <span key={`pad:${index}`} />
          ))}
          {Array.from({ length: dayCount }, (_, index) => {
            const key = `${month.year}-${pad(month.month)}-${pad(index + 1)}`;
            const day = days.get(key);
            if (!day)
              return (
                <span
                  key={key}
                  className={cn(
                    "grid aspect-[3/4] place-items-center rounded-xl text-sm font-bold text-ink-faint",
                    key === today && "border-2 border-dashed border-line",
                  )}
                >
                  {index + 1}
                </span>
              );
            const late = Number(day.lateSeconds) > 0;
            return (
              <Link
                key={key}
                to="/moments/$momentId"
                params={{ momentId: day.momentId }}
                aria-label={`${t("calendar.day", {
                  date: new Date(month.year, month.month - 1, index + 1).toLocaleDateString(
                    i18n.language,
                    { month: "long", day: "numeric" },
                  ),
                })} (${latenessText(i18n, day.lateSeconds)})`}
                className="relative aspect-[3/4] overflow-hidden rounded-xl bg-surface-3 text-sm font-black text-white"
                style={{ background: day.back?.dominantColor || undefined }}
                data-testid="moment-calendar-day"
              >
                {day.back?.url && (
                  <img
                    src={imageUrl(day.back, 56)}
                    alt=""
                    loading="lazy"
                    className="absolute inset-0 size-full object-cover"
                  />
                )}
                <span className="absolute inset-0 grid place-items-center bg-black/25">
                  {index + 1}
                </span>
                {!late && (
                  <span className="absolute top-0.5 right-0.5 text-[10px]" aria-hidden>
                    ⚡
                  </span>
                )}
              </Link>
            );
          })}
        </div>
        {calendar.isPending && <Skeleton className="mt-3 h-4 w-1/2 rounded-full" />}
        {calendar.data && calendar.data.days.length === 0 && (
          <p className="mt-3 text-center text-sm font-bold text-ink-soft">{t("calendar.empty")}</p>
        )}
      </section>
    </Page>
  );
}
