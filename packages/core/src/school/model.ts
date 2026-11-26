import { school, toMillis } from "@tokuly/api";
import type { i18n as I18n } from "i18next";
import { zonedMoment } from "../format/zoned";
import { clock, dayLabel } from "./dates";

/** Whether a listed class actually takes place (cancelled and moved-away ones are listed to show what changed). */
export function takesPlace(item: school.ScheduledClass): boolean {
  return (
    item.status !== school.ClassStatus.CANCELLED && item.status !== school.ClassStatus.MOVED_AWAY
  );
}

/** The name of a class's change under `school:day.status`; `undefined` for a class as planned. */
export function classStatusKey(status: school.ClassStatus): string | undefined {
  switch (status) {
    case school.ClassStatus.CANCELLED:
      return "cancelled";
    case school.ClassStatus.REPLACED:
      return "replaced";
    case school.ClassStatus.EXTRA:
      return "extra";
    case school.ClassStatus.MOVED_AWAY:
      return "movedAway";
    case school.ClassStatus.MOVED_HERE:
      return "movedHere";
    default:
      return undefined;
  }
}

/** The colour a scheduled class is drawn in: its own, else its subject's. */
export function classColor(item: Pick<school.ScheduledClass, "color" | "subject">): string {
  return item.color || item.subject?.color || "#ddd6ee";
}

/** Ink or white, whichever reads on a colour (`#rrggbb`). */
export function inkOn(color: string): string {
  const hex = /^#([0-9a-f]{6})$/i.exec(color)?.[1];
  if (!hex) return "#17122b";
  const [r, g, b] = [0, 2, 4].map((at) => Number.parseInt(hex.slice(at, at + 2), 16) / 255) as [
    number,
    number,
    number,
  ];
  return 0.299 * r + 0.587 * g + 0.114 * b > 0.6 ? "#17122b" : "#ffffff";
}

/** "あと12分" until a moment `ms` away; hours when it is further. */
export function untilText(i18n: I18n, ms: number): string {
  const minutes = Math.max(0, Math.ceil(ms / 60_000));
  return minutes < 60
    ? i18n.t("school:now.minutesLeft", { count: minutes })
    : i18n.t("school:now.hoursLeft", { count: Math.floor(minutes / 60), minutes: minutes % 60 });
}

/** When a scheduled class starts: "2限 9:50〜" today, "5/12(火) 2限 9:50〜" on another day. */
export function classWhen(i18n: I18n, item: school.ScheduledClass, today: string): string {
  const period = `${i18n.t("school:day.period", { period: item.period })} ${clock(item.startMinute)}〜`;
  return item.date === today ? period : `${dayLabel(item.date, i18n.language)} ${period}`;
}

/** The class still in progress at `nowMs` (the server's answer may be older than the bell). */
export function currentClass(
  now: school.ClassNow | undefined,
  nowMs: number,
): school.ScheduledClass | undefined {
  return now?.current && toMillis(now.current.endsAt) > nowMs ? now.current : undefined;
}

const URGENCY_KEY: Partial<Record<school.AssignmentUrgency, string>> = {
  [school.AssignmentUrgency.OVERDUE]: "overdue",
  [school.AssignmentUrgency.TODAY]: "today",
  [school.AssignmentUrgency.TOMORROW]: "tomorrow",
  [school.AssignmentUrgency.THIS_WEEK]: "thisWeek",
};

/** The name of an assignment's urgency under `school:assignments.urgency`; `undefined` when it is not soon. */
export function urgencyKey(urgency: school.AssignmentUrgency): string | undefined {
  return URGENCY_KEY[urgency];
}

/** An exam's countdown in words: "あと5日" before it, "テスト期間中 (あと3日)" during it. */
export function examCountdown(
  i18n: I18n,
  exam: Pick<school.Exam, "status" | "daysUntilStart" | "daysLeft">,
): string {
  if (exam.status === school.ExamStatus.FINISHED) return i18n.t("school:exams.finished");
  if (exam.status === school.ExamStatus.ONGOING)
    return i18n.t("school:exams.ongoing", { count: exam.daysLeft });
  return exam.daysUntilStart === 0
    ? i18n.t("school:exams.startsToday")
    : i18n.t("school:exams.daysUntil", { count: exam.daysUntilStart });
}

/** One line of "what is due": an exam that is on or coming up, or an open assignment. */
export type DueItem =
  | { at: number; exam: school.Exam; assignment?: undefined }
  | { at: number; exam?: undefined; assignment: school.Assignment };

/**
 * The exams that are on or coming up and the open assignments, soonest
 * first, in one list. An exam counts from the start of its first day on the
 * account's calendar (`zone`).
 */
export function dueItems(
  exams: readonly school.Exam[],
  assignments: readonly school.Assignment[],
  zone: string,
): DueItem[] {
  return [
    ...exams
      .filter((exam) => exam.status !== school.ExamStatus.FINISHED)
      .map((exam): DueItem => ({ at: zonedMoment(exam.startDate, 0, zone), exam })),
    ...assignments.map((assignment): DueItem => ({ at: toMillis(assignment.dueAt), assignment })),
  ].sort((a, b) => a.at - b.at);
}
