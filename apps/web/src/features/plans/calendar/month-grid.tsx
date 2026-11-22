import { ChevronLeft, ChevronRight } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { addMonths, formatMonth, monthGrid, monthOf, todayYmd, weekdayLabels } from "../dates";

export interface DayCell {
  /** `YYYY-MM-DD`. */
  day: string;
  /** The day belongs to the month on show (not the tail of a neighbour). */
  inMonth: boolean;
  isToday: boolean;
}

/**
 * A month page: a header to move between months and a grid of days, each a
 * button. What a day looks like and what a tap does is up to the caller —
 * the calendar screen draws what is on, the date picker toggles candidates.
 */
export function MonthGrid({
  month,
  onMonthChange,
  onPick,
  cellClassName,
  renderCell,
  isPressed,
  isDisabled,
  dayLabel,
  minMonth,
  headerAction,
}: {
  /** `YYYY-MM`. */
  month: string;
  onMonthChange(month: string): void;
  onPick(day: string): void;
  /** Extra classes for a day's button (its state colours). */
  cellClassName?(cell: DayCell): string | undefined;
  /** What goes under the day number. */
  renderCell?(cell: DayCell): ReactNode;
  /** The day is chosen (`aria-pressed`). */
  isPressed?(day: string): boolean;
  isDisabled?(day: string): boolean;
  /** The accessible name of a day's button, e.g. "12月24日(火)、予定2件". */
  dayLabel(cell: DayCell): string;
  /** The earliest month that can be shown. */
  minMonth?: string;
  headerAction?: ReactNode;
}) {
  const { t, i18n } = useTranslation("plans");
  const today = todayYmd();
  const weeks = monthGrid(month);
  return (
    <div>
      <div className="mb-2 flex items-center gap-1">
        <button
          type="button"
          aria-label={t("calendar.prev")}
          disabled={Boolean(minMonth && month <= minMonth)}
          onClick={() => onMonthChange(addMonths(month, -1))}
          className="grid size-10 place-items-center rounded-full hover:bg-surface-2 disabled:opacity-30"
        >
          <ChevronLeft className="size-5" />
        </button>
        <p className="min-w-0 flex-1 text-center text-lg font-black" aria-live="polite">
          {formatMonth(month, i18n.language)}
        </p>
        {headerAction}
        <button
          type="button"
          aria-label={t("calendar.next")}
          onClick={() => onMonthChange(addMonths(month, 1))}
          className="grid size-10 place-items-center rounded-full hover:bg-surface-2"
        >
          <ChevronRight className="size-5" />
        </button>
      </div>
      <div
        className="grid grid-cols-7 gap-1 text-center text-xs font-black text-ink-faint"
        aria-hidden
      >
        {weekdayLabels(i18n.language).map((label, index) => (
          <span
            key={label + String(index)}
            className={cn("py-1", index === 0 && "text-pink", index === 6 && "text-sky")}
          >
            {label}
          </span>
        ))}
      </div>
      <div className="grid grid-cols-7 gap-1">
        {weeks.flat().map((day) => {
          const cell: DayCell = { day, inMonth: monthOf(day) === month, isToday: day === today };
          const pressed = isPressed?.(day);
          return (
            <button
              key={day}
              type="button"
              aria-label={dayLabel(cell)}
              aria-pressed={pressed}
              aria-current={cell.isToday ? "date" : undefined}
              disabled={isDisabled?.(day)}
              onClick={() => onPick(day)}
              className={cn(
                "flex aspect-square min-h-11 flex-col items-center justify-start gap-0.5 rounded-xl border-2 border-transparent p-1 text-sm font-bold transition-colors disabled:opacity-30",
                !cell.inMonth && "text-ink-faint",
                cell.isToday && "border-brand",
                cellClassName?.(cell),
              )}
            >
              <span className="leading-none">{Number(day.slice(8))}</span>
              {renderCell?.(cell)}
            </button>
          );
        })}
      </div>
    </div>
  );
}
