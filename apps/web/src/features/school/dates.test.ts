import { school } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  addDays,
  clock,
  clockInput,
  daysBetween,
  isYmd,
  layoutWeekdays,
  mondayOf,
  parseClock,
  studyTime,
  timerText,
  weekdayOf,
  ymd,
} from "./dates";

describe("calendar dates", () => {
  it("formats a local date without going through UTC", () => {
    expect(ymd(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
    expect(ymd(new Date(2026, 11, 31, 0, 0))).toBe("2026-12-31");
  });

  it("adds days across months, years and leap days", () => {
    expect(addDays("2026-01-31", 1)).toBe("2026-02-01");
    expect(addDays("2026-01-01", -1)).toBe("2025-12-31");
    expect(addDays("2028-02-28", 1)).toBe("2028-02-29");
    expect(daysBetween("2026-03-01", "2026-03-15")).toBe(14);
    expect(daysBetween("2026-03-15", "2026-03-01")).toBe(-14);
  });

  it("numbers weekdays from Monday and finds the Monday of a week", () => {
    expect(weekdayOf("2026-11-23")).toBe(school.Weekday.MONDAY);
    expect(weekdayOf("2026-11-22")).toBe(school.Weekday.SUNDAY);
    expect(mondayOf("2026-11-22")).toBe("2026-11-16");
    expect(mondayOf("2026-11-23")).toBe("2026-11-23");
  });

  it("tells real dates from impossible ones", () => {
    expect(isYmd("2026-02-28")).toBe(true);
    expect(isYmd("2026-02-30")).toBe(false);
    expect(isYmd("2026-2-3")).toBe(false);
    expect(isYmd("")).toBe(false);
  });
});

describe("times of day", () => {
  it("shows minutes after midnight as a clock", () => {
    expect(clock(530)).toBe("8:50");
    expect(clockInput(530)).toBe("08:50");
    expect(clock(0)).toBe("0:00");
  });

  it("reads a time input back, and rejects what is not a time", () => {
    expect(parseClock("08:50")).toBe(530);
    expect(parseClock("23:59")).toBe(1439);
    expect(parseClock("24:00")).toBeUndefined();
    expect(parseClock("8:75")).toBeUndefined();
    expect(parseClock("")).toBeUndefined();
  });
});

describe("durations", () => {
  it("draws a running clock", () => {
    expect(timerText(0)).toBe("00:00");
    expect(timerText(247)).toBe("04:07");
    expect(timerText(3723)).toBe("1:02:03");
    expect(timerText(-5)).toBe("00:00");
  });

  it("says study time the way people do", () => {
    expect(studyTime(0, "ja")).toBe("0分");
    expect(studyTime(30, "ja")).toBe("1分未満");
    expect(studyTime(45 * 60, "ja")).toBe("45分");
    expect(studyTime(80 * 60, "ja")).toBe("1時間20分");
    expect(studyTime(2 * 3600, "ja")).toBe("2時間");
    expect(studyTime(80 * 60, "en")).toBe("1h 20m");
    expect(studyTime(30, "en")).toBe("<1m");
  });
});

describe("layout weekdays", () => {
  it("is Monday to Friday when the layout names none", () => {
    expect(layoutWeekdays(undefined)).toEqual([1, 2, 3, 4, 5]);
  });

  it("keeps the week's order whatever order the layout lists them in", () => {
    expect(
      layoutWeekdays({
        weekdays: [school.Weekday.SATURDAY, school.Weekday.MONDAY],
      } as school.PeriodLayout),
    ).toEqual([school.Weekday.MONDAY, school.Weekday.SATURDAY]);
  });
});
