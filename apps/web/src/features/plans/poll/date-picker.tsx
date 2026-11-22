import { useState } from "react";
import { useTranslation } from "react-i18next";
import { MonthGrid } from "../calendar/month-grid";
import { formatDay, monthOf, todayYmd } from "../dates";

/**
 * Candidate days picked on a month page: tap a day to add it, tap again to
 * take it out. Days that have passed cannot be chosen.
 */
export function DatePicker({
  value,
  onChange,
  taken,
}: {
  value: string[];
  onChange(days: string[]): void /** Days that are candidates already (shown, not selectable). */;
  taken?: string[];
}) {
  const { t, i18n } = useTranslation("plans");
  const today = todayYmd();
  const [month, setMonth] = useState(() => monthOf(value[0] ?? today));
  const chosen = new Set(value);
  const existing = new Set(taken ?? []);
  return (
    <MonthGrid
      month={month}
      onMonthChange={setMonth}
      minMonth={monthOf(today)}
      onPick={(day) =>
        onChange(chosen.has(day) ? value.filter((other) => other !== day) : [...value, day].sort())
      }
      isPressed={(day) => chosen.has(day)}
      isDisabled={(day) => day < today || existing.has(day)}
      dayLabel={(cell) =>
        `${formatDay(cell.day, i18n.language)}${existing.has(cell.day) ? `、${t("poll.form.already")}` : ""}`
      }
      cellClassName={(cell) =>
        chosen.has(cell.day)
          ? "border-ink bg-lime text-[#17122b] shadow-pop-sm"
          : existing.has(cell.day)
            ? "bg-surface-3"
            : "bg-surface hover:bg-surface-2"
      }
      renderCell={(cell) =>
        chosen.has(cell.day) ? <span className="text-xs leading-none font-black">✓</span> : null
      }
    />
  );
}
