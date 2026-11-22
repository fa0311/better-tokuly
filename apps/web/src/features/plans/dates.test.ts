import { describe, expect, it } from "vitest";
import {
  addDays,
  addMonths,
  combine,
  daysBetween,
  formatTime,
  fromDateTimeInput,
  monthGrid,
  monthOf,
  parseYmd,
  planTimeZone,
  timeOf,
  toDateTimeInput,
  todayYmd,
  ymd,
} from "./dates";

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

  it("joins a day and a time on the account's clock", () => {
    const at = combine("2026-11-22", "18:05")!;
    expect(ymd(at)).toBe("2026-11-22");
    expect(timeOf(at)).toBe("18:05");
    expect(combine("2026-11-22", "6pm")).toBeNull();
    expect(combine("2026-11-31", "18:05")).toBeNull();
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
 * running them is in: CI runs them in UTC, and `pnpm test` is also run with
 * `TZ=America/Los_Angeles` (see the web README).
 */
describe("the account's time zone, not the device's", () => {
  // Without a session the account's zone is Asia/Tokyo (UTC+9).
  it("reads the day and the time of an instant on the account's clock", () => {
    // 15:30 UTC on the 21st is 00:30 on the 22nd in Tokyo — and still the 21st in Los Angeles.
    const instant = new Date("2026-11-21T15:30:00Z");
    expect(ymd(instant)).toBe("2026-11-22");
    expect(timeOf(instant)).toBe("00:30");
    expect(toDateTimeInput(instant)).toBe("2026-11-22T00:30");
    expect(todayYmd(instant)).toBe("2026-11-22");
    expect(formatTime(instant, "ja")).toBe("0:30");
  });

  it("turns a day and a time into the instant the account means", () => {
    expect(combine("2026-11-22", "18:00")?.toISOString()).toBe("2026-11-22T09:00:00.000Z");
    expect(fromDateTimeInput("2026-11-22T00:30")?.toISOString()).toBe("2026-11-21T15:30:00.000Z");
    expect(fromDateTimeInput("soon")).toBeNull();
  });

  it("round-trips through another zone when one is named", () => {
    const zone = "America/Los_Angeles";
    const at = combine("2026-11-22", "18:00", zone)!;
    expect(at.toISOString()).toBe("2026-11-23T02:00:00.000Z");
    expect(ymd(at, zone)).toBe("2026-11-22");
    expect(timeOf(at, zone)).toBe("18:00");
    // The same instant is the next morning in Tokyo.
    expect(ymd(at)).toBe("2026-11-23");
    expect(timeOf(at)).toBe("11:00");
  });

  it("names the account's zone in requests", () => {
    expect(planTimeZone()).toBe("Asia/Tokyo");
  });

  it("keeps calendar arithmetic free of any zone", () => {
    expect(addDays("2026-03-08", 1)).toBe("2026-03-09");
    expect(daysBetween("2026-03-07", "2026-03-09")).toBe(2);
    expect(monthGrid("2026-11")[0]?.[0]).toBe("2026-11-01");
    expect(monthGrid("2026-11").flat()).toContain("2026-11-30");
  });
});
