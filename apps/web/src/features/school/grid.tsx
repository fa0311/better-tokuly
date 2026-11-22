import type { school } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { clock, layoutWeekdays, weekdayOf } from "./dates";
import { cellName, entryColor, type GridEntry, type Slot, slotKey, slots } from "./grid-model";
import { inkOn } from "./parts";

/**
 * The weekly grid: weekdays across, periods down. A slot shows its class in
 * the subject's colour; in a term with A/B weeks a slot can hold two, drawn
 * as halves. `onCell` makes the slots buttons (to edit, or to look closer).
 */
export function TimetableGrid({
  layout,
  entries,
  onCell,
  editing,
  highlightToday = true,
}: {
  layout: school.PeriodLayout | undefined;
  entries: GridEntry[];
  onCell?(weekday: school.Weekday, period: school.Period, slot: Slot | undefined): void;
  /** Draws empty slots as targets. */
  editing?: boolean;
  highlightToday?: boolean;
}) {
  const { t } = useTranslation("school");
  const weekdays = layoutWeekdays(layout);
  const periods = layout?.periods ?? [];
  const bySlot = slots(entries);
  const today = weekdayOf(useAccountToday());

  return (
    <table
      aria-label={t("timetable.grid")}
      className="w-full table-fixed border-separate border-spacing-1"
    >
      <thead>
        <tr>
          <td className="w-9" />
          {weekdays.map((weekday) => (
            <th
              key={weekday}
              scope="col"
              className="p-0"
              aria-current={highlightToday && weekday === today ? "date" : undefined}
            >
              <span
                className={cn(
                  "grid h-7 place-items-center rounded-full text-xs font-black",
                  highlightToday && weekday === today ? "bg-ink text-surface" : "text-ink-soft",
                )}
              >
                {t(`weekdays.short.${weekday}`)}
              </span>
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {periods.map((period) => (
          <tr key={period.number}>
            <th scope="row" className="p-0 text-center leading-tight">
              <span className="block text-sm font-black">{period.label || period.number}</span>
              <span className="block text-[10px] font-bold text-ink-faint">
                {clock(period.startMinute)}
              </span>
            </th>
            {weekdays.map((weekday) => {
              const slot = bySlot.get(slotKey(weekday, period.number));
              const label = t("timetable.slotLabel", {
                weekday: t(`weekdays.long.${weekday}`),
                period: period.number,
              });
              const name = `${label} ${slotText(slot, t("timetable.weekA"), t("timetable.weekB")) || t("timetable.emptySlot")}`;
              return (
                <td key={weekday} className="h-16 p-0 sm:h-[4.5rem]">
                  {onCell ? (
                    <button
                      type="button"
                      aria-label={name}
                      onClick={() => onCell(weekday, period, slot)}
                      className="pressable block h-16 w-full rounded-xl text-left sm:h-[4.5rem]"
                    >
                      <SlotContent slot={slot} editing={editing} />
                    </button>
                  ) : (
                    <div className="h-16 sm:h-[4.5rem]">
                      <span className="sr-only">{name}</span>
                      <SlotContent slot={slot} editing={editing} />
                    </div>
                  )}
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function slotText(slot: Slot | undefined, weekA: string, weekB: string): string {
  if (!slot) return "";
  if (slot.every) return slot.every.subject?.name ?? "";
  return [
    slot.a && `${weekA} ${slot.a.subject?.name ?? ""}`,
    slot.b && `${weekB} ${slot.b.subject?.name ?? ""}`,
  ]
    .filter(Boolean)
    .join(" / ");
}

function SlotContent({ slot, editing }: { slot: Slot | undefined; editing?: boolean }) {
  if (!slot || (!slot.every && !slot.a && !slot.b)) {
    return (
      <span
        aria-hidden
        className={cn(
          "block size-full rounded-xl border-2 border-dashed",
          editing ? "border-ink-faint bg-surface" : "border-line",
        )}
      />
    );
  }
  if (slot.every) return <ClassTile entry={slot.every} />;
  return (
    <span aria-hidden className="flex size-full flex-col gap-0.5">
      <HalfTile entry={slot.a} mark="A" />
      <HalfTile entry={slot.b} mark="B" />
    </span>
  );
}

function ClassTile({ entry }: { entry: GridEntry }) {
  const color = entryColor(entry);
  const room = entry.room || entry.subject?.room;
  return (
    <span
      aria-hidden
      className="flex size-full flex-col justify-between overflow-hidden rounded-xl border-2 border-ink px-1.5 py-1"
      style={{ background: color, color: inkOn(color) }}
    >
      <span className="line-clamp-2 text-[13px] font-black leading-tight break-all">
        {cellName(entry.subject)}
      </span>
      {room && <span className="truncate text-[10px] font-bold opacity-80">{room}</span>}
    </span>
  );
}

function HalfTile({ entry, mark }: { entry: GridEntry | undefined; mark: "A" | "B" }) {
  if (!entry) {
    return (
      <span className="flex min-h-0 flex-1 items-center rounded-lg border-2 border-dashed border-line px-1 text-[10px] font-black text-ink-faint">
        {mark}
      </span>
    );
  }
  const color = entryColor(entry);
  return (
    <span
      className="flex min-h-0 flex-1 items-center gap-1 overflow-hidden rounded-lg border-2 border-ink px-1"
      style={{ background: color, color: inkOn(color) }}
    >
      <span className="text-[10px] font-black opacity-70">{mark}</span>
      <span className="truncate text-xs font-black">{cellName(entry.subject)}</span>
    </span>
  );
}
