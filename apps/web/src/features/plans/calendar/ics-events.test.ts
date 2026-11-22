import { create, fromDate, type MessageInitShape, plans } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { planIcsEvent } from "../plan/model";
import { calendarIcsEvents, entryUid } from "./ics-events";

const ORIGIN = "https://tokuly.example";

function entry(init: MessageInitShape<typeof plans.CalendarEntrySchema>): plans.CalendarEntry {
  return create(plans.CalendarEntrySchema, init);
}

function planAt(id: string, start: Date, end?: Date): plans.Plan {
  return create(plans.PlanSchema, {
    id,
    title: "カラオケ",
    emoji: "🎤",
    time: {
      start: { case: "startAt", value: fromDate(start) },
      end: end ? { case: "endAt", value: fromDate(end) } : { case: undefined },
    },
    place: { name: "カラオケ館" },
  });
}

describe("calendar file events of a month", () => {
  it("keeps a plan's UID when the plan moves to another day", () => {
    const before = entry({
      kind: plans.CalendarEntryKind.PLAN,
      refId: "p1",
      date: "2026-11-25",
      title: "カラオケ",
      path: "/plans/p1",
    });
    const after = entry({
      kind: plans.CalendarEntryKind.PLAN,
      refId: "p1",
      date: "2026-12-03",
      title: "カラオケ",
      path: "/plans/p1",
    });
    expect(entryUid(before)).toBe("plan-p1");
    expect(entryUid(after)).toBe(entryUid(before));
    const [first] = calendarIcsEvents([before], new Map(), "ja", ORIGIN);
    const [second] = calendarIcsEvents([after], new Map(), "ja", ORIGIN);
    expect(second?.uid).toBe(first?.uid);
    expect(second?.date).toBe("2026-12-03");
  });

  it("writes a plan exactly as its own page does: same UID, with the end and the place", () => {
    const start = new Date(Date.UTC(2026, 10, 25, 8, 0));
    const end = new Date(Date.UTC(2026, 10, 25, 11, 0));
    const plan = planAt("p1", start, end);
    const item = entry({
      kind: plans.CalendarEntryKind.PLAN,
      refId: "p1",
      date: "2026-11-25",
      startAt: fromDate(start),
      title: "カラオケ",
      path: "/plans/p1",
    });
    const [event] = calendarIcsEvents([item], new Map([["p1", plan]]), "ja", ORIGIN);
    expect(event).toEqual(planIcsEvent(plan, `${ORIGIN}/plans/p1`));
    expect(event?.uid).toBe("plan-p1");
    expect(event?.end).toEqual(end);
    expect(event?.location).toBe("カラオケ館");
  });

  it("falls back to the entry when the plan could not be read", () => {
    const start = new Date(Date.UTC(2026, 10, 25, 8, 0));
    const item = entry({
      kind: plans.CalendarEntryKind.PLAN,
      refId: "p1",
      date: "2026-11-25",
      startAt: fromDate(start),
      title: "カラオケ",
      emoji: "🎤",
    });
    const [event] = calendarIcsEvents([item], new Map(), "ja", ORIGIN);
    expect(event).toMatchObject({ uid: "plan-p1", title: "🎤 カラオケ", start, date: undefined });
  });

  it("gives polls and countdowns one UID each, and only yearly things their day", () => {
    expect(
      entryUid(
        entry({ kind: plans.CalendarEntryKind.POLL_DEADLINE, refId: "q1", date: "2026-11-24" }),
      ),
    ).toBe("poll-q1");
    expect(
      entryUid(entry({ kind: plans.CalendarEntryKind.COUNTDOWN, refId: "c1", date: "2026-12-04" })),
    ).toBe("countdown-c1");
    const birthday = (date: string) =>
      entryUid(entry({ kind: plans.CalendarEntryKind.BIRTHDAY, date, user: { id: "u1" } }));
    expect(birthday("2027-04-02")).toBe("birthday-u1-2027-04-02");
    expect(birthday("2028-04-02")).not.toBe(birthday("2027-04-02"));
    // Two milestones of one counter are two events.
    const milestone = (date: string) =>
      entryUid(entry({ kind: plans.CalendarEntryKind.ANNIVERSARY, refId: "a1", date }));
    expect(milestone("2026-11-24")).not.toBe(milestone("2027-02-17"));
  });

  it("names a birthday by the server's label", () => {
    const item = entry({
      kind: plans.CalendarEntryKind.BIRTHDAY,
      date: "2027-04-02",
      user: { id: "u1" },
      label: { ja: "あおいの誕生日", en: "Aoi's birthday" },
    });
    expect(calendarIcsEvents([item], new Map(), "ja", ORIGIN)[0]?.title).toBe("🎂 あおいの誕生日");
    expect(calendarIcsEvents([item], new Map(), "en", ORIGIN)[0]?.title).toBe("🎂 Aoi's birthday");
  });
});
