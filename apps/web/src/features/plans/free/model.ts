import { type common, fromDate, plans, toDate, toMillis } from "@tokuly/api";
import type { TFunction } from "i18next";
import { compactDuration } from "#/lib/format";
import { formatDay, formatTime, todayYmd, ymd } from "../dates";
import type { PlanDraft } from "../draft";

/** The status is running at `now` (the server's `free_now` is as old as the response). */
export function isRunning(status: plans.FreeStatus, now: number): boolean {
  return toMillis(status.startsAt) <= now;
}

/** The window has run out: the status is gone, even if nothing said so. */
export function hasEnded(status: plans.FreeStatus, now: number): boolean {
  return toMillis(status.endsAt) <= now;
}

/** A time of day, with the day in front when it is not today: "18:00" / "11月28日(土) 0:00". */
export function clock(date: Date, language: string, now: number): string {
  const time = formatTime(date, language);
  return ymd(date) === todayYmd(new Date(now))
    ? time
    : `${formatDay(ymd(date), language, new Date(now))} ${time}`;
}

/** "18:00まで" while it runs, "土曜 0:00から" before it starts. */
export function whenText(
  status: plans.FreeStatus,
  now: number,
  language: string,
  t: TFunction<"plans">,
): string {
  const [start, end] = [toDate(status.startsAt), toDate(status.endsAt)];
  if (!start || !end) return "";
  return isRunning(status, now)
    ? t("free.until", { time: clock(end, language, now) })
    : t("free.from", { time: clock(start, language, now) });
}

/** "あと2時間で自動で消えるよ": every status says that it ends by itself. */
export function expiryText(status: plans.FreeStatus, now: number, t: TFunction<"plans">): string {
  return t("free.expiresIn", {
    duration: compactDuration(Math.max(0, toMillis(status.endsAt) - now)),
  });
}

/** The short badge on a face: "今ひま", or when it starts. */
export function badgeText(
  status: plans.FreeStatus,
  now: number,
  language: string,
  t: TFunction<"plans">,
): string {
  const start = toDate(status.startsAt);
  return isRunning(status, now) || !start
    ? t("free.nowBadge")
    : t("free.from", { time: clock(start, language, now) });
}

/**
 * A plan for the two, from a friend's status: used when the server's own
 * draft (which comes with the answer) is not at hand any more.
 */
export function draftFromStatus(status: plans.FreeStatus, now: number): PlanDraft {
  const start = toDate(status.startsAt);
  const startAt = start && start.getTime() > now ? start : undefined;
  return {
    inviteeIds: status.user ? [status.user.id] : [],
    // "To be decided" unless the window still lies ahead.
    time: startAt
      ? ({
          start: { case: "startAt", value: fromDate(startAt) },
          end: { case: undefined },
          timeZone: "",
        } as plans.PlanTime)
      : undefined,
    place: status.areaName
      ? ({ name: status.areaName, placeId: "", address: "" } as plans.PlanPlace)
      : undefined,
  };
}

/** Per account: several people may use one browser, and their audiences are their own. */
const audienceKey = (userId: string) => `tokuly.plans.freeAudience.${userId}`;

/** The audience this account chose last time, so setting a status again stays two taps. */
export function rememberedAudience(userId: string): {
  audience: plans.FreeAudience;
  chosenIds: string[];
} {
  try {
    const stored = JSON.parse(window.localStorage.getItem(audienceKey(userId)) ?? "null") as {
      audience?: number;
      chosenIds?: string[];
    } | null;
    if (
      stored &&
      (stored.audience === plans.FreeAudience.BEST_FRIENDS ||
        stored.audience === plans.FreeAudience.CHOSEN)
    ) {
      return {
        audience: stored.audience,
        chosenIds: Array.isArray(stored.chosenIds) ? stored.chosenIds : [],
      };
    }
  } catch {
    // Unreadable storage is the same as nothing remembered.
  }
  return { audience: plans.FreeAudience.FRIENDS, chosenIds: [] };
}

export function rememberAudience(
  userId: string,
  audience: plans.FreeAudience,
  chosen: common.UserSummary[],
): void {
  try {
    window.localStorage.setItem(
      audienceKey(userId),
      JSON.stringify({ audience, chosenIds: chosen.map((user) => user.id) }),
    );
  } catch {
    // Not remembered, then.
  }
}
