import { describe, expect, it } from "vitest";
import {
  addDays,
  addMonths,
  combine,
  daysBetween,
  formatDay,
  formatTime,
  monthGrid,
  monthOf,
  parseYmd,
  timeOf,
  todayYmd,
  ymd,
} from "./dates";

const TOKYO = "Asia/Tokyo";
const LOS_ANGELES = "America/Los_Angeles";

describe("days", () => {
  it("round-trips a day and refuses days that do not exist", () => {
    expect(parseYmd("2026-02-28")?.getDate()).toBe(28);
    expect(addDays("2026-02-28", 0)).toBe("2026-02-28");
    expect(parseYmd("2026-02-30")).toBeNull();
    expect(parseYmd("tomorrow")).toBeNull();
  });

  it("moves across month and year ends", () => {
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addDays("2028-03-01", -1)).toBe("2028-02-29");
    expect(addMonths("2026-12", 1)).toBe("2027-01");
    expect(addMonths("2026-01", -1)).toBe("2025-12");
    expect(monthOf("2026-11-22")).toBe("2026-11");
  });

  it("counts whole days in both directions", () => {
    expect(daysBetween("2026-11-22", "2026-12-25")).toBe(33);
    expect(daysBetween("2026-12-25", "2026-11-22")).toBe(-33);
    expect(daysBetween("2026-11-22", "2026-11-22")).toBe(0);
  });

  it("joins a day and a time on the zone's clock", () => {
    const at = combine("2026-11-22", "18:05", TOKYO)!;
    expect(ymd(at, TOKYO)).toBe("2026-11-22");
    expect(timeOf(at, TOKYO)).toBe("18:05");
    expect(combine("2026-11-22", "6pm", TOKYO)).toBeNull();
    expect(combine("2026-11-31", "18:05", TOKYO)).toBeNull();
  });
});

describe("monthGrid", () => {
  it("lays a month out in full weeks starting on Sunday", () => {
    const weeks = monthGrid("2026-11");
    expect(weeks).toHaveLength(5);
    expect(weeks[0]![0]).toBe("2026-11-01"); // November 2026 starts on a Sunday.
    expect(weeks[4]![6]).toBe("2026-12-05");
    expect(weeks.every((week) => week.length === 7)).toBe(true);
  });

  it("includes the days of the previous month that share the first week", () => {
    const weeks = monthGrid("2026-12");
    expect(weeks[0]![0]).toBe("2026-11-29");
    expect(weeks.flat()).toContain("2026-12-31");
    expect(weeks).toHaveLength(5);
  });

  it("gives a month that spans six weeks all of them", () => {
    expect(monthGrid("2026-08")).toHaveLength(6);
  });
});

/**
 * Plans live on the account's clock. These hold whatever zone the machine
 * running them is in: CI runs them in UTC, and the tests are also run with
 * `TZ=America/Los_Angeles`.
 */
describe("the zone given, not the device's", () => {
  it("reads the day and the time of an instant on the zone's clock", () => {
    // 15:30 UTC on the 21st is 00:30 on the 22nd in Tokyo — and still the 21st in Los Angeles.
    const instant = new Date("2026-11-21T15:30:00Z");
    expect(ymd(instant, TOKYO)).toBe("2026-11-22");
    expect(timeOf(instant, TOKYO)).toBe("00:30");
    expect(todayYmd(TOKYO, instant)).toBe("2026-11-22");
    expect(todayYmd(LOS_ANGELES, instant)).toBe("2026-11-21");
    expect(formatTime(instant, "ja", TOKYO)).toBe("0:30");
  });

  it("turns a day and a time into the instant the zone means", () => {
    expect(combine("2026-11-22", "18:00", TOKYO)?.toISOString()).toBe("2026-11-22T09:00:00.000Z");
    const at = combine("2026-11-22", "18:00", LOS_ANGELES)!;
    expect(at.toISOString()).toBe("2026-11-23T02:00:00.000Z");
    expect(ymd(at, LOS_ANGELES)).toBe("2026-11-22");
    expect(timeOf(at, LOS_ANGELES)).toBe("18:00");
    // The same instant is the next morning in Tokyo.
    expect(ymd(at, TOKYO)).toBe("2026-11-23");
    expect(timeOf(at, TOKYO)).toBe("11:00");
  });

  it("names the year only when it is not the year of today", () => {
    expect(formatDay("2026-12-24", "ja", "2026-11-22")).not.toContain("2026");
    expect(formatDay("2027-01-02", "ja", "2026-11-22")).toContain("2027");
  });

  it("keeps calendar arithmetic free of any zone", () => {
    expect(addDays("2026-03-08", 1)).toBe("2026-03-09");
    expect(daysBetween("2026-03-07", "2026-03-09")).toBe(2);
    expect(monthGrid("2026-11")[0]?.[0]).toBe("2026-11-01");
    expect(monthGrid("2026-11").flat()).toContain("2026-11-30");
  });
});
