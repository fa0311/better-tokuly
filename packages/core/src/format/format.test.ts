import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { compactAge, compactDuration, distanceMeters, formatDistance, kmh } from "./index";

const i18n = testI18n();

describe("format", () => {
  const now = 1_800_000_000_000;

  it("shows NOW for fresh positions, then growing units", () => {
    expect(compactAge(i18n, now - 3_000, now)).toBe("NOW");
    expect(compactAge(i18n, now - 30_000, now)).toBe("たった今");
    expect(compactAge(i18n, now - 12 * 60_000, now)).toBe("12分前");
    expect(compactAge(i18n, now - 3 * 3_600_000, now)).toBe("3時間前");
    expect(compactAge(i18n, now - 50 * 3_600_000, now)).toBe("2日前");
    expect(compactAge(i18n, now + 5_000, now)).toBe("NOW");
  });

  it("formats stay durations compactly", () => {
    expect(compactDuration("ja", 20_000)).toBe("1分");
    expect(compactDuration("ja", 45 * 60_000)).toBe("45分");
    expect(compactDuration("ja", 3 * 3_600_000)).toBe("3時間");
    expect(compactDuration("ja", 72 * 3_600_000)).toBe("3日");
    expect(compactDuration("en", 3 * 3_600_000)).toBe("3h");
  });

  it("converts speed and distance", () => {
    expect(kmh(5)).toBe(18);
    expect(formatDistance(847)).toBe("850 m");
    expect(formatDistance(8_546)).toBe("8.5 km");
    expect(formatDistance(8_546_100)).toBe("8,546 km");
    expect(formatDistance(1609.344, "mi")).toBe("1.0 mi");
  });

  it("measures great-circle distance", () => {
    const tokyo = { lat: 35.681236, lng: 139.767125 };
    const shibuya = { lat: 35.658034, lng: 139.701636 };
    const d = distanceMeters(tokyo, shibuya);
    expect(d).toBeGreaterThan(6200);
    expect(d).toBeLessThan(6600);
    expect(distanceMeters(tokyo, tokyo)).toBe(0);
  });
});
