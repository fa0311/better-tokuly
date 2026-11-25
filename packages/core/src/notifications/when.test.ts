import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { dayBucket, formatWhen, minutesToTime, timeToMinutes } from "./when";

const i18n = testI18n();

describe("notification times", () => {
  // Built from local-time parts, so the buckets hold in any time zone.
  const now = new Date(2026, 10, 22, 15, 0).getTime();
  const at = (day: number, hour: number) => new Date(2026, 10, day, hour, 0).getTime();

  it("sorts entries into today, yesterday, this week and earlier by the calendar", () => {
    expect(dayBucket(at(22, 0), now)).toBe("today");
    expect(dayBucket(at(21, 23), now)).toBe("yesterday");
    expect(dayBucket(at(21, 0), now)).toBe("yesterday");
    expect(dayBucket(at(20, 23), now)).toBe("week");
    expect(dayBucket(at(16, 0), now)).toBe("week");
    expect(dayBucket(at(15, 23), now)).toBe("earlier");
  });

  it("shows an age for the last week and a date after that", () => {
    expect(formatWhen(i18n, now - 3 * 60_000, now, "ja")).toBe("3分前");
    expect(formatWhen(i18n, at(20, 15), now, "ja")).toBe("2日前");
    expect(formatWhen(i18n, at(3, 9), now, "ja")).toBe("11月3日");
    expect(formatWhen(i18n, new Date(2025, 11, 31, 9, 0).getTime(), now, "ja")).toBe(
      "2025年12月31日",
    );
  });

  it("converts between minutes after midnight and a time input's value", () => {
    expect(minutesToTime(0)).toBe("00:00");
    expect(minutesToTime(1320)).toBe("22:00");
    expect(minutesToTime(425)).toBe("07:05");
    expect(timeToMinutes("22:00")).toBe(1320);
    expect(timeToMinutes("07:05:30")).toBe(425);
    expect(timeToMinutes("")).toBeNull();
    expect(timeToMinutes("24:00")).toBeNull();
  });
});
