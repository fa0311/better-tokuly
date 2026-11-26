import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { watchToday, zonedMoment } from "./account-time";

// The device is in Los Angeles; the account lives in Tokyo, seventeen hours ahead in winter.
const DEVICE_ZONE = "America/Los_Angeles";
const TOKYO = "Asia/Tokyo";
let deviceZoneBefore: string | undefined;

beforeEach(() => {
  deviceZoneBefore = process.env.TZ;
  process.env.TZ = DEVICE_ZONE;
});

afterEach(() => {
  process.env.TZ = deviceZoneBefore;
  vi.useRealTimers();
});

describe("watchToday", () => {
  it("reports today at once, then the new date when the account's midnight passes", () => {
    vi.useFakeTimers();
    // 23:59:30 on Tuesday in Tokyo; mid-morning on Tuesday in Los Angeles.
    vi.setSystemTime(zonedMoment("2026-11-24", 23 * 60 + 59, TOKYO) + 30_000);
    const seen: string[] = [];
    const stop = watchToday(TOKYO, (date) => seen.push(date));
    expect(seen).toEqual(["2026-11-24"]);

    vi.advanceTimersByTime(20_000);
    expect(seen).toEqual(["2026-11-24"]);
    vi.advanceTimersByTime(15_000);
    expect(seen).toEqual(["2026-11-24", "2026-11-25"]);

    // It keeps watching: the next midnight is reported too.
    vi.advanceTimersByTime(24 * 3_600_000);
    expect(seen).toEqual(["2026-11-24", "2026-11-25", "2026-11-26"]);

    stop();
    vi.advanceTimersByTime(48 * 3_600_000);
    expect(seen).toHaveLength(3);
  });

  it("does not move on at the device's midnight", () => {
    vi.useFakeTimers();
    // 23:59 on Monday in Los Angeles is Tuesday afternoon in Tokyo.
    vi.setSystemTime(zonedMoment("2026-11-23", 23 * 60 + 59, DEVICE_ZONE));
    const seen: string[] = [];
    const stop = watchToday(TOKYO, (date) => seen.push(date));
    vi.advanceTimersByTime(5 * 60_000);
    expect(seen).toEqual(["2026-11-24"]);
    stop();
  });
});
