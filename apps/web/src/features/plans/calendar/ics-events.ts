import { plans, toDate } from "@tokuly/api";
import { localized } from "../countdown/model";
import type { IcsEvent } from "../ics";
import { planIcsEvent } from "../plan/model";

/** The emoji an entry is drawn with when it has none of its own, by kind. */
export const KIND_EMOJI: Record<number, string> = {
  [plans.CalendarEntryKind.PLAN]: "📅",
  [plans.CalendarEntryKind.POLL_DEADLINE]: "🗳️",
  [plans.CalendarEntryKind.COUNTDOWN]: "⏳",
  [plans.CalendarEntryKind.ANNIVERSARY]: "💞",
  [plans.CalendarEntryKind.BIRTHDAY]: "🎂",
};

/**
 * The UID of an entry in a calendar file: one per item, whatever day it is
 * on, so an item that was moved replaces itself when the file is imported
 * again. Only what comes back on other days carries its day: a birthday
 * (every year) and the milestones of one counter (100 days, a year...).
 */
export function entryUid(entry: plans.CalendarEntry): string {
  switch (entry.kind) {
    case plans.CalendarEntryKind.PLAN:
      // The same UID the plan page writes ("add to calendar").
      return `plan-${entry.refId}`;
    case plans.CalendarEntryKind.POLL_DEADLINE:
      return `poll-${entry.refId}`;
    case plans.CalendarEntryKind.COUNTDOWN:
      return `countdown-${entry.refId}`;
    case plans.CalendarEntryKind.ANNIVERSARY:
      return `anniversary-${entry.refId}-${entry.date}`;
    default:
      return `birthday-${entry.user?.id ?? "unknown"}-${entry.date}`;
  }
}

/** What an entry is called: its own title, or the server's description of it ("〇〇の誕生日"). */
export function entryTitle(entry: plans.CalendarEntry, language: string): string {
  return entry.title || localized(entry.label, language);
}

/**
 * A month's entries as calendar-file events. A plan found in `plansById` is
 * written from the plan itself — with its end, its place and its notes,
 * exactly as its own page exports it; the calendar entry alone only knows
 * when it starts.
 */
export function calendarIcsEvents(entries: plans.CalendarEntry[], plansById: ReadonlyMap<string, plans.Plan>, language: string, origin: string): IcsEvent[] {
  return entries.map((entry) => {
    const url = entry.path ? `${origin}${entry.path}` : undefined;
    const plan = entry.kind === plans.CalendarEntryKind.PLAN ? plansById.get(entry.refId) : undefined;
    if (plan) return planIcsEvent(plan, url ?? `${origin}/plans/${plan.id}`);
    return {
      uid: entryUid(entry),
      title: `${entry.emoji || KIND_EMOJI[entry.kind] || ""} ${entryTitle(entry, language)}`.trim(),
      date: entry.startAt ? undefined : entry.date,
      start: toDate(entry.startAt),
      description: (entry.title && localized(entry.label, language)) || undefined,
      url,
    };
  });
}
