import { describe, expect, it } from "vitest";
import {
  combine,
  formatTime,
  fromDateTimeInput,
  planTimeZone,
  timeOf,
  toDateTimeInput,
  todayYmd,
  ymd,
} from "./dates";

/**
 * Plans live on the account's clock (the rules themselves are tested in
 * `@tokuly/core/plans`). These hold whatever zone the machine running them is
 * in: CI runs them in UTC, and `pnpm test` is also run with
 * `TZ=America/Los_Angeles` (see the web README).
 */
describe("the account's time zone, not the device's", () => {
  // Without a session the account's zone is Asia/Tokyo (UTC+9).
  it("reads the day and the time of an instant on the account's clock", () => {
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

  it("names the account's zone in requests", () => {
    expect(planTimeZone()).toBe("Asia/Tokyo");
  });
});
