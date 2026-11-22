import { type common, fromDate, type MessageInitShape, plans, toDate } from "@tokuly/api";
import type { TFunction } from "i18next";
import {
  combine,
  daysBetween,
  formatDay,
  formatTime,
  localTimeZone,
  timeOf,
  todayYmd,
  ymd,
} from "../dates";
import type { PlanDraft } from "../draft";
import type { IcsEvent } from "../ics";

/** How the time of a plan is given: not yet, a day, or a day and a time. */
export type WhenKind = "tbd" | "date" | "time";

/** What the plan form holds while it is being filled in. */
export interface PlanFormValue {
  title: string;
  emoji: string;
  description: string;
  when: WhenKind;
  date: string;
  time: string;
  hasEnd: boolean;
  endDate: string;
  endTime: string;
  placeName: string;
  address: string;
  /** Kept from a place chosen elsewhere (the map, a friend's rough area) while its name stays. */
  placeExtra?: { name: string; location?: common.LatLng; placeId: string };
  invitees: common.UserSummary[];
  guestsCanInvite: boolean;
  /** Empty = no limit. */
  capacity: string;
  remindDayBefore: boolean;
  remindMinutesBefore: number;
  currency: string;
}

export const EMPTY_FORM: PlanFormValue = {
  title: "",
  emoji: "",
  description: "",
  when: "tbd",
  date: "",
  time: "",
  hasEnd: false,
  endDate: "",
  endTime: "",
  placeName: "",
  address: "",
  invitees: [],
  guestsCanInvite: true,
  capacity: "",
  remindDayBefore: true,
  remindMinutesBefore: 60,
  currency: "JPY",
};

/** The time part of the form from a plan's (or a draft's) time. */
function timeFields(
  time: plans.PlanTime | undefined,
): Pick<PlanFormValue, "when" | "date" | "time" | "hasEnd" | "endDate" | "endTime"> {
  const none = {
    when: "tbd" as const,
    date: "",
    time: "",
    hasEnd: false,
    endDate: "",
    endTime: "",
  };
  if (!time) return none;
  if (time.start.case === "startDate") {
    const endDate = time.end.case === "endDate" ? time.end.value : "";
    return { ...none, when: "date", date: time.start.value, hasEnd: Boolean(endDate), endDate };
  }
  if (time.start.case === "startAt") {
    const start = toDate(time.start.value)!;
    const end = time.end.case === "endAt" ? toDate(time.end.value) : undefined;
    return {
      when: "time",
      date: ymd(start),
      time: timeOf(start),
      hasEnd: Boolean(end),
      endDate: end ? ymd(end) : "",
      endTime: end ? timeOf(end) : "",
    };
  }
  return none;
}

export function formFromPlan(plan: plans.Plan): PlanFormValue {
  return {
    ...EMPTY_FORM,
    title: plan.title,
    emoji: plan.emoji,
    description: plan.description,
    ...timeFields(plan.time),
    placeName: plan.place?.name ?? "",
    address: plan.place?.address ?? "",
    placeExtra: plan.place
      ? { name: plan.place.name, location: plan.place.location, placeId: plan.place.placeId }
      : undefined,
    guestsCanInvite: plan.guestsCanInvite,
    capacity: plan.capacity > 0 ? String(plan.capacity) : "",
    remindDayBefore: plan.remindDayBefore,
    remindMinutesBefore: plan.remindMinutesBefore,
    currency: plan.currency || "JPY",
  };
}

/** A new plan's form from what is known already; `friends` turns invitee IDs into people. */
export function formFromDraft(
  draft: PlanDraft | null,
  friends: common.UserSummary[],
): PlanFormValue {
  if (!draft) return EMPTY_FORM;
  const wanted = new Set(draft.inviteeIds ?? []);
  return {
    ...EMPTY_FORM,
    title: draft.title ?? "",
    emoji: draft.emoji ?? "",
    description: draft.description ?? "",
    ...timeFields(draft.time),
    placeName: draft.place?.name ?? "",
    address: draft.place?.address ?? "",
    placeExtra: draft.place
      ? { name: draft.place.name, location: draft.place.location, placeId: draft.place.placeId }
      : undefined,
    invitees: friends.filter((friend) => wanted.has(friend.id)),
  };
}

/** Why the form cannot be sent yet, as a key of the `plan.form.errors` strings; `null` when it can. */
export function formProblem(
  value: PlanFormValue,
): "title" | "date" | "time" | "end" | "capacity" | null {
  if (!value.title.trim()) return "title";
  if (value.when !== "tbd" && !value.date) return "date";
  if (value.when === "time" && !combine(value.date, value.time)) return "time";
  if (value.when !== "tbd" && value.hasEnd) {
    if (value.when === "date" && value.endDate && value.endDate < value.date) return "end";
    if (value.when === "time") {
      const [start, end] = [
        combine(value.date, value.time),
        combine(value.endDate || value.date, value.endTime),
      ];
      if (!end || !start || end < start) return "end";
    }
  }
  if (value.capacity && !(Number.isInteger(Number(value.capacity)) && Number(value.capacity) >= 1))
    return "capacity";
  return null;
}

/** The form's time as the API takes it; an empty message means "to be decided". */
export function toPlanTime(value: PlanFormValue): MessageInitShape<typeof plans.PlanTimeSchema> {
  const timeZone = localTimeZone();
  if (value.when === "date") {
    return {
      start: { case: "startDate", value: value.date },
      end:
        value.hasEnd && value.endDate
          ? { case: "endDate", value: value.endDate }
          : { case: undefined },
      timeZone,
    };
  }
  if (value.when === "time") {
    const start = combine(value.date, value.time);
    const end = value.hasEnd ? combine(value.endDate || value.date, value.endTime) : null;
    if (start)
      return {
        start: { case: "startAt", value: fromDate(start) },
        end: end ? { case: "endAt", value: fromDate(end) } : { case: undefined },
        timeZone,
      };
  }
  return { timeZone };
}

/** The form's place; an empty name removes the place. */
export function toPlanPlace(value: PlanFormValue): MessageInitShape<typeof plans.PlanPlaceSchema> {
  const name = value.placeName.trim();
  const kept = value.placeExtra && value.placeExtra.name === name ? value.placeExtra : undefined;
  return {
    name,
    address: name ? value.address.trim() : "",
    location: kept?.location,
    placeId: kept?.placeId ?? "",
  };
}

export function toCreateRequest(
  value: PlanFormValue,
): MessageInitShape<typeof plans.CreatePlanRequestSchema> {
  return {
    title: value.title.trim(),
    emoji: value.emoji,
    description: value.description.trim(),
    time: value.when === "tbd" ? undefined : toPlanTime(value),
    place: value.placeName.trim() ? toPlanPlace(value) : undefined,
    inviteeIds: value.invitees.map((user) => user.id),
    guestsCanInvite: value.guestsCanInvite,
    capacity: value.capacity ? Number(value.capacity) : 0,
    remindDayBefore: value.remindDayBefore,
    remindMinutesBefore: value.remindMinutesBefore,
    currency: value.currency,
  };
}

/**
 * What changed between two states of the form, as an update. The time and
 * the place are only sent when they changed: each such change tells
 * everyone in the plan.
 */
export function toUpdateRequest(
  planId: string,
  before: PlanFormValue,
  value: PlanFormValue,
): MessageInitShape<typeof plans.UpdatePlanRequestSchema> {
  const same = (a: unknown, b: unknown) =>
    JSON.stringify(a, (_key, item) => (typeof item === "bigint" ? item.toString() : item)) ===
    JSON.stringify(b, (_key, item) => (typeof item === "bigint" ? item.toString() : item));
  return {
    planId,
    title: value.title.trim(),
    emoji: value.emoji,
    description: value.description.trim(),
    time: same(toPlanTime(before), toPlanTime(value)) ? undefined : toPlanTime(value),
    place: same(toPlanPlace(before), toPlanPlace(value)) ? undefined : toPlanPlace(value),
    guestsCanInvite: value.guestsCanInvite,
    capacity: value.capacity ? Number(value.capacity) : 0,
    remindDayBefore: value.remindDayBefore,
    remindMinutesBefore: value.remindMinutesBefore,
    currency: value.currency,
  };
}

/** "12月24日(火) 18:00〜21:00", "12月24日(火)", or "日にち未定". */
export function planTimeText(
  time: plans.PlanTime | undefined,
  language: string,
  t: TFunction<"plans">,
  now: Date = new Date(),
): string {
  if (time?.start.case === "startDate") {
    const end =
      time.end.case === "endDate" && time.end.value !== time.start.value
        ? `〜${formatDay(time.end.value, language, now)}`
        : "";
    return `${formatDay(time.start.value, language, now)}${end}`;
  }
  if (time?.start.case === "startAt") {
    const start = toDate(time.start.value)!;
    const end = time.end.case === "endAt" ? toDate(time.end.value) : undefined;
    const startText = `${formatDay(ymd(start), language, now)} ${formatTime(start, language)}`;
    if (!end) return `${startText}〜`;
    return `${startText}〜${ymd(end) === ymd(start) ? "" : `${formatDay(ymd(end), language, now)} `}${formatTime(end, language)}`;
  }
  return t("plan.tbd");
}

/** The day a plan is on (where the person is), or nothing while it is to be decided. */
export function planDay(time: plans.PlanTime | undefined): string | undefined {
  if (time?.start.case === "startDate") return time.start.value;
  if (time?.start.case === "startAt") return ymd(toDate(time.start.value)!);
  return undefined;
}

/** "今日", "あした", "あと3日" for a plan that lies ahead. */
export function untilText(
  time: plans.PlanTime | undefined,
  t: TFunction<"plans">,
  now: Date = new Date(),
): string | undefined {
  const day = planDay(time);
  if (!day) return undefined;
  const days = daysBetween(todayYmd(now), day);
  if (days < 0) return undefined;
  return days === 0
    ? t("plan.today")
    : days === 1
      ? t("plan.tomorrow")
      : t("plan.inDays", { count: days });
}

/** The caller edits the plan, invites and removes people. */
export function isHostLike(plan: plans.Plan): boolean {
  return plan.myRole === plans.PlanRole.HOST || plan.myRole === plans.PlanRole.CO_HOST;
}

/** "Anyone going" works on the checklist and the expenses. */
export function canWork(plan: plans.Plan): boolean {
  return (
    plan.status === plans.PlanStatus.ACTIVE &&
    (isHostLike(plan) || plan.myRsvp === plans.Rsvp.GOING)
  );
}

export function canInvite(plan: plans.Plan): boolean {
  return plan.status === plans.PlanStatus.ACTIVE && (isHostLike(plan) || plan.guestsCanInvite);
}

/** A plan as a calendar event, for "add to my calendar". */
export function planIcsEvent(plan: plans.Plan, url: string): IcsEvent {
  const time = plan.time;
  return {
    uid: `plan-${plan.id}`,
    title: `${plan.emoji ? `${plan.emoji} ` : ""}${plan.title}`,
    date: time?.start.case === "startDate" ? time.start.value : undefined,
    endDate: time?.end.case === "endDate" ? time.end.value : undefined,
    start: time?.start.case === "startAt" ? toDate(time.start.value) : undefined,
    end: time?.end.case === "endAt" ? toDate(time.end.value) : undefined,
    description: plan.description || undefined,
    location: plan.place
      ? [plan.place.name, plan.place.address].filter(Boolean).join(" ")
      : undefined,
    url,
  };
}

/** The reminder lead times offered, in minutes (0 = off). */
export const REMINDER_MINUTES = [0, 10, 30, 60, 120, 180] as const;
