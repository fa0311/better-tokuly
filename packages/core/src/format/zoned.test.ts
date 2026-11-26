import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { untilNextDay, zonedDate, zonedMoment, zonedTime } from "./zoned";

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
});

/** Tuesday 2026-11-24 08:00 in Tokyo, which is still Monday afternoon in Los Angeles. */
const TUESDAY_MORNING_TOKYO = Date.UTC(2026, 10, 23, 23, 0);

describe("the account's calendar on a device in another time zone", () => {
  it("really runs with a device zone that differs from the account's", () => {
    expect(new Date(TUESDAY_MORNING_TOKYO).getDate()).toBe(23);
    expect(new Date(TUESDAY_MORNING_TOKYO).getHours()).toBe(15);
  });

  it("says the account's date, not the device's", () => {
    expect(zonedDate(TUESDAY_MORNING_TOKYO, TOKYO)).toBe("2026-11-24");
    expect(zonedDate(TUESDAY_MORNING_TOKYO, DEVICE_ZONE)).toBe("2026-11-23");
  });

  it("says the time of day on the account's clock", () => {
    expect(zonedTime(TUESDAY_MORNING_TOKYO, TOKYO)).toEqual({
      date: "2026-11-24",
      minute: 8 * 60,
      second: 0,
    });
    // Midnight is 0:00, never 24:00.
    expect(zonedTime(Date.UTC(2026, 10, 23, 15, 0, 5), TOKYO)).toEqual({
      date: "2026-11-24",
      minute: 0,
      second: 5,
    });
  });

  it("finds the moment a date and time of day name in a zone", () => {
    expect(zonedMoment("2026-11-24", 8 * 60 + 50, TOKYO)).toBe(Date.UTC(2026, 10, 23, 23, 50));
    expect(zonedMoment("2026-11-24", 0, "UTC")).toBe(Date.UTC(2026, 10, 24));
    // Either side of a daylight-saving change (Los Angeles falls back on 2026-11-01).
    expect(zonedMoment("2026-10-31", 12 * 60, DEVICE_ZONE)).toBe(Date.UTC(2026, 9, 31, 19, 0));
    expect(zonedMoment("2026-11-01", 12 * 60, DEVICE_ZONE)).toBe(Date.UTC(2026, 10, 1, 20, 0));
  });

  it("round-trips a moment through a zone's clock", () => {
    for (const zone of [TOKYO, DEVICE_ZONE, "Asia/Kolkata", "Pacific/Kiritimati"]) {
      const shown = zonedTime(TUESDAY_MORNING_TOKYO, zone);
      expect(zonedMoment(shown.date, shown.minute, zone), zone).toBe(TUESDAY_MORNING_TOKYO);
    }
  });

  it("falls back to the default zone for one the runtime does not know", () => {
    expect(zonedDate(TUESDAY_MORNING_TOKYO, "Mars/Olympus_Mons")).toBe("2026-11-24");
  });

  it("counts down to the account's midnight, across months and a 25-hour day", () => {
    expect(untilNextDay(TUESDAY_MORNING_TOKYO, TOKYO)).toBe(16 * 3_600_000);
    expect(untilNextDay(Date.UTC(2026, 10, 30, 14, 59), TOKYO)).toBe(60_000);
    // The day Los Angeles sets its clocks back has 25 hours.
    expect(untilNextDay(zonedMoment("2026-11-01", 0, DEVICE_ZONE), DEVICE_ZONE)).toBe(
      25 * 3_600_000,
    );
  });
});
