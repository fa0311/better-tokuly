import { create, fromDate, plans } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import {
  EMPTY_FORM,
  formFromPlan,
  formProblem,
  type PlanFormValue,
  planTimeText,
  toCreateRequest,
  toUpdateRequest,
  untilText,
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
    expect(formProblem(form({}), TOKYO)).toBeNull();
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
