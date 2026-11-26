import { type plans, toDate } from "@tokuly/api";
import type { IcsEvent } from "../ics";

/**
 * The web's own part of a plan's rules; the rest (the form, its requests,
 * the texts of a plan's time, who may do what) is `@tokuly/core/plans`.
 */

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
