import { create, fromDate, plans } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import {
  canInvite,
  EMPTY_FORM,
  formFromPlan,
  formProblem,
  type PlanFormValue,
  planTimeText,
  toCreateRequest,
  toUpdateRequest,
  untilText,
  withEndTime,
} from "./plan";

const TOKYO = "Asia/Tokyo";
const i18n = testI18n("ja");

const form = (value: Partial<PlanFormValue>): PlanFormValue => ({
  ...EMPTY_FORM,
  title: "カラオケ",
  ...value,
});

const timed = (startIso: string, endIso?: string) =>
  create(plans.PlanTimeSchema, {
    start: { case: "startAt", value: fromDate(new Date(startIso)) },
    end: endIso ? { case: "endAt", value: fromDate(new Date(endIso)) } : { case: undefined },
    timeZone: TOKYO,
  });

describe("the plan form", () => {
  it("says what is missing before it can be sent", () => {
    expect(formProblem(form({ title: " " }), TOKYO)).toBe("title");
    expect(formProblem(form({ when: "date" }), TOKYO)).toBe("date");
    expect(formProblem(form({ when: "time", date: "2026-12-24", time: "7pm" }), TOKYO)).toBe(
      "time",
    );
    expect(
      formProblem(
        form({ when: "time", date: "2026-12-24", time: "18:00", hasEnd: true, endTime: "17:00" }),
        TOKYO,
      ),
    ).toBe("end");
    expect(formProblem(form({ capacity: "0" }), TOKYO)).toBe("capacity");
    expect(
      formProblem(
        form({ when: "date", date: "2026-12-24", hasEnd: true, endDate: "2026-12-24" }),
        TOKYO,
      ),
    ).toBeNull();
    expect(formProblem(form({}), TOKYO)).toBeNull();
  });

  it("wants a timed plan to end after it starts, as the server does", () => {
    const at = (time: string, endTime: string, date = "2026-12-24", zone = TOKYO) =>
      formProblem(form({ when: "time", date, time, hasEnd: true, endTime }), zone);
    expect(at("18:00", "18:00")).toBe("end");
    expect(at("18:00", "18:05")).toBeNull();
    // 02:00 and 02:30 are skipped in Los Angeles that night: both are 03:00.
    expect(at("02:00", "02:30", "2026-03-08", "America/Los_Angeles")).toBe("end");
  });

  it("takes an end time earlier than the start as the next day: a night out past midnight", () => {
    const night = form({ when: "time", date: "2026-12-24", time: "22:00", hasEnd: true });
    const late = withEndTime(night, "01:00");
    expect(late).toMatchObject({ endDate: "2026-12-25", endTime: "01:00" });
    expect(formProblem(late, TOKYO)).toBeNull();
    // Later the same day stays that day; an end day already after the start is kept.
    expect(withEndTime(night, "23:30")).toMatchObject({ endDate: "" });
    expect(withEndTime(late, "23:30")).toMatchObject({ endDate: "2026-12-25" });
    const picked = { ...night, endDate: "2026-12-26" };
    expect(withEndTime(picked, "01:00")).toMatchObject({ endDate: "2026-12-26" });
  });

  it("sends a day and a time as the instant the zone means, naming the zone", () => {
    const request = toCreateRequest(
      form({ when: "time", date: "2026-12-24", time: "18:00" }),
      TOKYO,
    );
    expect(request.time?.start?.case).toBe("startAt");
    expect(request.time?.timeZone).toBe(TOKYO);
    const time = create(plans.PlanTimeSchema, request.time);
    expect(time.start.case === "startAt" && time.start.value.seconds).toBe(
      BigInt(Date.parse("2026-12-24T09:00:00Z") / 1000),
    );
    expect(toCreateRequest(form({}), TOKYO).time).toBeUndefined();
  });

  it("leaves the time and the place out of an update that did not change them", () => {
    const plan = create(plans.PlanSchema, {
      id: "p1",
      title: "カラオケ",
      time: timed("2026-12-24T09:00:00Z"),
      place: { name: "駅前" },
    });
    const before = formFromPlan(plan, TOKYO);
    expect(before).toMatchObject({ when: "time", date: "2026-12-24", time: "18:00" });
    const renamed = toUpdateRequest("p1", before, { ...before, title: "カラオケ!" }, TOKYO);
    expect(renamed.time).toBeUndefined();
    expect(renamed.place).toBeUndefined();
    const moved = toUpdateRequest("p1", before, { ...before, time: "19:00" }, TOKYO);
    expect(moved.time).toBeDefined();
  });

  it("sends only what changed, so a co-host's edit at the same time is not undone", () => {
    const plan = create(plans.PlanSchema, {
      id: "p1",
      title: "カラオケ",
      capacity: 6,
      remindMinutesBefore: 30,
      time: timed("2026-12-24T09:00:00Z"),
    });
    const before = formFromPlan(plan, TOKYO);
    expect(toUpdateRequest("p1", before, { ...before, title: " カラオケ! " }, TOKYO)).toEqual({
      planId: "p1",
      title: "カラオケ!",
    });
    expect(toUpdateRequest("p1", before, { ...before, capacity: "" }, TOKYO)).toEqual({
      planId: "p1",
      capacity: 0,
    });
    expect(toUpdateRequest("p1", before, before, TOKYO)).toEqual({ planId: "p1" });
  });
});

describe("the time of a plan in words", () => {
  const now = new Date("2026-11-22T03:00:00Z");

  it("reads the day and the time on the zone's clock", () => {
    expect(
      planTimeText(i18n, timed("2026-12-24T09:00:00Z", "2026-12-24T12:00:00Z"), TOKYO, now),
    ).toBe("12月24日(木) 18:00〜21:00");
    expect(planTimeText(i18n, undefined, TOKYO, now)).toBe("日にち未定");
  });

  it("counts the days until it on the zone's calendar", () => {
    // 23:30 on the 22nd in Tokyo: a plan at 00:30 is tomorrow there.
    const late = new Date("2026-11-22T14:30:00Z");
    expect(untilText(i18n, timed("2026-11-22T15:30:00Z"), TOKYO, late)).toBe("あした");
    expect(untilText(i18n, timed("2026-11-22T15:30:00Z"), "UTC", late)).toBe("今日");
    expect(untilText(i18n, timed("2026-11-20T15:30:00Z"), TOKYO, late)).toBeUndefined();
  });
});

describe("who may invite", () => {
  const plan = (myRole: plans.PlanRole, myRsvp: plans.Rsvp, guestsCanInvite = true) =>
    create(plans.PlanSchema, { status: plans.PlanStatus.ACTIVE, myRole, myRsvp, guestsCanInvite });

  it("is the hosts, and guests who have not said no while the plan lets them", () => {
    expect(canInvite(plan(plans.PlanRole.HOST, plans.Rsvp.GOING, false))).toBe(true);
    expect(canInvite(plan(plans.PlanRole.GUEST, plans.Rsvp.MAYBE))).toBe(true);
    expect(canInvite(plan(plans.PlanRole.GUEST, plans.Rsvp.INVITED))).toBe(true);
    expect(canInvite(plan(plans.PlanRole.GUEST, plans.Rsvp.GOING, false))).toBe(false);
  });

  it("is not a guest who declined (the server refuses them)", () => {
    expect(canInvite(plan(plans.PlanRole.GUEST, plans.Rsvp.DECLINED))).toBe(false);
  });
});
