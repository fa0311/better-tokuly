import { describe, expect, it } from "vitest";
import {
  addDays,
  addMonths,
  combine,
  daysBetween,
  monthGrid,
  monthOf,
  parseYmd,
  timeOf,
  ymd,
} from "./dates";

describe("days", () => {
  it("round-trips a day and refuses days that do not exist", () => {
    expect(ymd(parseYmd("2026-02-28")!)).toBe("2026-02-28");
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

  it("joins a day and a time", () => {
    const at = combine("2026-11-22", "18:05")!;
    expect(ymd(at)).toBe("2026-11-22");
    expect(timeOf(at)).toBe("18:05");
    expect(combine("2026-11-22", "6pm")).toBeNull();
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
