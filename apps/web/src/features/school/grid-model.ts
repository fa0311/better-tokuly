import { school } from "@tokuly/api";

/** What the grid needs of a class; a friend's entries and the caller's own both fit. */
export type GridEntry = Pick<
  school.TimetableEntry,
  "id" | "weekday" | "period" | "weekPattern" | "subject" | "room" | "teacher" | "color" | "memo"
>;

/** The classes of one slot: one for every week, or one for week A and/or one for week B. */
export interface Slot {
  every?: GridEntry;
  a?: GridEntry;
  b?: GridEntry;
}

export function slotKey(weekday: school.Weekday, period: number): string {
  return `${weekday}:${period}`;
}

/** The grid's entries by slot. */
export function slots(entries: GridEntry[]): Map<string, Slot> {
  const map = new Map<string, Slot>();
  for (const entry of entries) {
    const key = slotKey(entry.weekday, entry.period);
    const slot = map.get(key) ?? {};
    if (entry.weekPattern === school.WeekPattern.A) slot.a = entry;
    else if (entry.weekPattern === school.WeekPattern.B) slot.b = entry;
    else slot.every = entry;
    map.set(key, slot);
  }
  return map;
}

/** The class of a slot in weeks of `pattern` (EVERY: the every-week class only). */
export function entryFor(
  slot: Slot | undefined,
  pattern: school.WeekPattern,
): GridEntry | undefined {
  if (!slot) return undefined;
  if (pattern === school.WeekPattern.A) return slot.a ?? slot.every;
  if (pattern === school.WeekPattern.B) return slot.b ?? slot.every;
  return slot.every;
}

/**
 * Whether a class put into a slot with `pattern` pushes `existing` out: an
 * every-week class replaces everything, a week-A class replaces the
 * every-week class and the other week-A class (the server's rule).
 */
export function collides(
  existing: GridEntry,
  weekday: school.Weekday,
  period: number,
  pattern: school.WeekPattern,
): boolean {
  if (existing.weekday !== weekday || existing.period !== period) return false;
  const every = school.WeekPattern.EVERY;
  const own = existing.weekPattern || every;
  return pattern === every || own === every || own === pattern;
}

/** The colour a class is drawn in: its own, else its subject's. */
export function entryColor(entry: Pick<GridEntry, "color" | "subject">): string {
  return entry.color || entry.subject?.color || "#ddd6ee";
}

/** The name for a narrow cell: the subject's short form when it has one. */
export function cellName(subject: school.Subject | undefined): string {
  return subject?.shortName || subject?.name || "";
}
