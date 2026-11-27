import { describe, expect, it } from "vitest";
import { dayRange, formatDay, monthGrid, parseDay, shiftDay, shiftMonth } from "./dates";

describe("days", () => {
  it("parses real dates only", () => {
    expect(formatDay(parseDay("2026-11-22")!)).toBe("2026-11-22");
    expect(parseDay("2026-02-30")).toBeNull();
    expect(parseDay("2026-13-01")).toBeNull();
    expect(parseDay("yesterday")).toBeNull();
  });

  it("steps across months and years", () => {
    expect(shiftDay("2026-11-30", 1)).toBe("2026-12-01");
    expect(shiftDay("2027-01-01", -1)).toBe("2026-12-31");
    expect(shiftDay("2028-02-28", 1)).toBe("2028-02-29");
  });

  it("spans a day from midnight to midnight in the account's time zone", () => {
    const range = dayRange("2026-11-22", "Asia/Tokyo")!;
    expect(range.from.toISOString()).toBe("2026-11-21T15:00:00.000Z");
    expect(range.to.toISOString()).toBe("2026-11-22T15:00:00.000Z");
    expect(dayRange("2026-11-22", "UTC")?.from.toISOString()).toBe("2026-11-22T00:00:00.000Z");
    // A day a daylight-saving jump shortens is still one calendar day.
    const short = dayRange("2026-03-08", "America/New_York")!;
    expect((short.to.getTime() - short.from.getTime()) / 3_600_000).toBe(23);
    expect(dayRange("nope", "Asia/Tokyo")).toBeNull();
  });
});

describe("months", () => {
  it("steps across years", () => {
    expect(shiftMonth({ year: 2026, month: 12 }, 1)).toEqual({ year: 2027, month: 1 });
    expect(shiftMonth({ year: 2026, month: 1 }, -1)).toEqual({ year: 2025, month: 12 });
    expect(shiftMonth({ year: 2026, month: 5 }, -17)).toEqual({ year: 2024, month: 12 });
  });

  it("lays a month out in whole weeks from Sunday", () => {
    // November 2026 starts on a Sunday and has 30 days.
    const november = monthGrid({ year: 2026, month: 11 });
    expect(november).toHaveLength(5);
    expect(november[0]![0]).toBe("2026-11-01");
    expect(november[4]).toEqual(["2026-11-29", "2026-11-30", null, null, null, null, null]);
    // December 2026 starts on a Tuesday.
    const december = monthGrid({ year: 2026, month: 12 });
    expect(december[0]).toEqual([
      null,
      null,
      "2026-12-01",
      "2026-12-02",
      "2026-12-03",
      "2026-12-04",
      "2026-12-05",
    ]);
    expect(december.flat().filter(Boolean)).toHaveLength(31);
    expect(december.every((week) => week.length === 7)).toBe(true);
  });
});
