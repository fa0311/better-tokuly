import { Link } from "@tanstack/react-router";
import { toMillis } from "@tokuly/api";
import { classWhen, clock, currentClass, inkOn, untilText } from "@tokuly/core/school";
import { CalendarDays, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { useAccountNow } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useClassNow } from "./api";

/**
 * The class in progress and the next one, like a home-screen widget: the
 * subject in its colour, the room, and how long until the bell.
 */
export function ClassNowCard({ className }: { className?: string }) {
  const { t, i18n } = useTranslation("school");
  const { data, isPending } = useClassNow();
  // The account's clock: "today" is theirs, wherever the device is.
  const { ms: now, date: today } = useAccountNow(15_000);
  const until = (ms: number) => untilText(i18n, ms);

  if (isPending) return <Skeleton className={cn("h-36", className)} />;
  const current = currentClass(data, now);
  const next = data?.next;
  const color = current ? current.color || current.subject?.color || "#c6f24e" : undefined;

  return (
    <section
      aria-label={t("now.title")}
      className={cn("sticker overflow-hidden rounded-card bg-surface", className)}
    >
      {current ? (
        <div className="p-4" style={{ background: color, color: inkOn(color ?? "") }}>
          <p className="flex items-center gap-2 text-xs font-black">
            <span className="rounded-full bg-live px-2 py-0.5 text-white">{t("now.current")}</span>
            <span>
              {t("day.period", { period: current.period })} · 〜{clock(current.endMinute)}
            </span>
          </p>
          <p className="mt-1.5 truncate text-3xl font-black">
            {current.subject?.emoji} {current.subject?.name}
          </p>
          <p className="mt-0.5 flex items-center gap-2 text-sm font-bold">
            <span className="min-w-0 truncate">
              {[current.room, current.teacher].filter(Boolean).join(" · ") || " "}
            </span>
            <span className="ml-auto shrink-0">{until(toMillis(current.endsAt) - now)}</span>
          </p>
          <Progress from={toMillis(current.startsAt)} to={toMillis(current.endsAt)} now={now} />
        </div>
      ) : (
        <div className="p-4">
          <p className="text-xs font-black text-ink-soft">{t("now.title")}</p>
          <p className="mt-1 text-2xl font-black">
            {next ? (next.date === today ? t("now.break") : t("now.doneToday")) : t("now.nothing")}
          </p>
        </div>
      )}
      {next ? (
        <div className="flex items-center gap-3 border-t-2 border-ink px-4 py-3">
          <span
            className="size-3 shrink-0 rounded-full border-2 border-ink"
            style={{ background: next.color || next.subject?.color }}
          />
          <span className="min-w-0 flex-1">
            <span className="block text-xs font-black text-ink-soft">{t("now.next")}</span>
            <span className="block truncate font-black">
              {next.subject?.emoji} {next.subject?.name}
              {next.room && (
                <span className="ml-2 text-sm font-bold text-ink-soft">{next.room}</span>
              )}
            </span>
          </span>
          <span className="shrink-0 text-right text-sm font-bold">
            <span className="block">{classWhen(i18n, next, today)}</span>
            {next.date === today && (
              <span className="block text-xs text-ink-soft">
                {until(toMillis(next.startsAt) - now)}
              </span>
            )}
          </span>
        </div>
      ) : (
        !current && (
          <Link
            to="/school/timetable"
            className="flex items-center gap-2 border-t-2 border-line px-4 py-3 text-sm font-bold text-brand"
          >
            <CalendarDays className="size-4" />
            {t("now.openTimetable")}
          </Link>
        )
      )}
    </section>
  );
}

function Progress({ from, to, now }: { from: number; to: number; now: number }) {
  const done = Math.min(1, Math.max(0, (now - from) / Math.max(1, to - from)));
  return (
    <div className="mt-3 h-2.5 overflow-hidden rounded-full border-2 border-current bg-white/40">
      <div className="h-full bg-current" style={{ width: `${done * 100}%` }} />
    </div>
  );
}

/**
 * The school tile of the "じぶん" hub: leads to the school area and says
 * what class is on, or which one comes next.
 */
export function SchoolHubTile() {
  const { t, i18n } = useTranslation("school");
  const { data } = useClassNow();
  const { ms: now, date: today } = useAccountNow(30_000);
  const current = currentClass(data, now);
  const next = data?.next;
  const line = current
    ? t("now.tileCurrent", { subject: current.subject?.name ?? "", end: clock(current.endMinute) })
    : next
      ? t("now.tileNext", {
          subject: next.subject?.name ?? "",
          when: classWhen(i18n, next, today),
        })
      : t("now.tileEmpty");
  return (
    <Link
      to="/school"
      className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4"
    >
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-lime text-2xl">
        🏫
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-black">{t("title")}</span>
        <span className="block truncate text-sm font-bold text-ink-soft">{line}</span>
      </span>
      <ChevronRight className="size-6 shrink-0 text-ink-faint" />
    </Link>
  );
}
