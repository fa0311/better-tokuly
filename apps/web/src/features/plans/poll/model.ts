import { fromDate, type MessageInitShape, plans, toDate } from "@tokuly/api";
import { combine, formatDay, formatTime, ymd } from "../dates";

/** A time of day offered on every chosen date: `HH:MM`, optionally until `HH:MM`. */
export interface TimeRange {
  start: string;
  end: string;
}

/** The server takes at most this many candidates. */
export const MAX_SLOTS = 30;

/**
 * The candidates of a poll from what was picked: every chosen day as a whole
 * day, or every chosen day at each of the times.
 */
export function buildSlots(days: string[], times: TimeRange[] | null): MessageInitShape<typeof plans.PollSlotInputSchema>[] {
  const sorted = [...days].sort();
  if (!times) return sorted.map((day) => ({ start: { case: "date", value: day } }));
  const slots: MessageInitShape<typeof plans.PollSlotInputSchema>[] = [];
  const seen = new Set<string>();
  for (const day of sorted) {
    for (const time of times) {
      const start = combine(day, time.start);
      if (!start || seen.has(`${day} ${time.start}`)) continue;
      seen.add(`${day} ${time.start}`);
      const end = time.end ? combine(day, time.end) : null;
      slots.push({ start: { case: "startAt", value: fromDate(start) }, endAt: end && end > start ? fromDate(end) : undefined });
    }
  }
  return slots;
}

/** The day a candidate is on, where the person is. */
export function slotDay(slot: plans.PollSlot): string {
  if (slot.start.case === "date") return slot.start.value;
  return slot.start.case === "startAt" ? ymd(toDate(slot.start.value)!) : "";
}

/** "12月24日(火)" and, for a candidate with a time, "18:00〜21:00". */
export function slotLabel(slot: plans.PollSlot, language: string): { day: string; time?: string } {
  if (slot.start.case === "date") return { day: formatDay(slot.start.value, language) };
  if (slot.start.case !== "startAt") return { day: "" };
  const start = toDate(slot.start.value)!;
  const end = toDate(slot.endAt);
  return { day: formatDay(ymd(start), language), time: `${formatTime(start, language)}〜${end ? formatTime(end, language) : ""}` };
}

export function slotText(slot: plans.PollSlot, language: string): string {
  const label = slotLabel(slot, language);
  return label.time ? `${label.day} ${label.time}` : label.day;
}

export const ANSWER_MARK: Record<number, string> = { [plans.PollAnswer.YES]: "○", [plans.PollAnswer.MAYBE]: "△", [plans.PollAnswer.NO]: "×" };

/** A poll takes answers: open, and its deadline (if any) has not passed. */
export function takesAnswers(poll: plans.Poll, now: number): boolean {
  const deadline = toDate(poll.deadline);
  return poll.status === plans.PollStatus.OPEN && (!deadline || deadline.getTime() > now);
}
