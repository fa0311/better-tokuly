import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import {
  formatAge,
  formatClock,
  formatCount,
  parseClock,
  parseTimeParam,
  videoIdFromInput,
} from "./format";

describe("video formatting", () => {
  it("writes positions as clocks", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(7.9)).toBe("0:07");
    expect(formatClock(754)).toBe("12:34");
    expect(formatClock(3723)).toBe("1:02:03");
    expect(formatClock(Number.NaN)).toBe("0:00");
    expect(formatClock(-5)).toBe("0:00");
  });

  it("reads clocks people type", () => {
    expect(parseClock("0:00")).toBe(0);
    expect(parseClock("1:30")).toBe(90);
    expect(parseClock("1:02:03")).toBe(3723);
    expect(parseClock("75")).toBe(75);
    expect(parseClock(" 2:05 ")).toBe(125);
    expect(parseClock("1:75")).toBeNull();
    expect(parseClock("abc")).toBeNull();
    expect(parseClock("")).toBeNull();
    expect(parseClock("1:2:3:4")).toBeNull();
  });

  it("reads the ?t= of a deep link in every form", () => {
    expect(parseTimeParam("90")).toBe(90);
    expect(parseTimeParam("90s")).toBe(90);
    expect(parseTimeParam("1:30")).toBe(90);
    expect(parseTimeParam("1m30s")).toBe(90);
    expect(parseTimeParam("1h2m3s")).toBe(3723);
    expect(parseTimeParam("2m")).toBe(120);
    expect(parseTimeParam(undefined)).toBeNull();
    expect(parseTimeParam("soon")).toBeNull();
    expect(parseTimeParam("")).toBeNull();
  });

  it("keeps small counts exact and compacts big ones", () => {
    expect(formatCount("ja", 0)).toBe("0");
    expect(formatCount("ja", 9999n)).toBe("9,999");
    expect(formatCount("ja", 12_345)).toBe("1.2万");
    expect(formatCount("en", 12_345)).toBe("12.3K");
  });

  it("ages publication dates coarsely", () => {
    const i18n = testI18n("ja");
    const now = 1_800_000_000_000;
    expect(formatAge(i18n, now - 20_000, now)).toBe("たった今");
    expect(formatAge(i18n, now - 5 * 60_000, now)).toBe("5分前");
    expect(formatAge(i18n, now - 3 * 3_600_000, now)).toBe("3時間前");
    expect(formatAge(i18n, now - 2 * 86_400_000, now)).toBe("2日前");
    expect(formatAge(i18n, now - 15 * 86_400_000, now)).toBe("2週間前");
    expect(formatAge(i18n, now - 70 * 86_400_000, now)).toBe("2か月前");
    expect(formatAge(i18n, now - 800 * 86_400_000, now)).toBe("2年前");
  });

  it("finds the video ID in a pasted link or ID", () => {
    expect(videoIdFromInput("4c0H3LG8Y60")).toBe("4c0H3LG8Y60");
    expect(videoIdFromInput("https://tokuly.example/watch/4c0H3LG8Y60?t=30")).toBe("4c0H3LG8Y60");
    expect(videoIdFromInput("  /watch/01a200cf-886f-70f5-aa02-24095e5b378f ")).toBe(
      "01a200cf-886f-70f5-aa02-24095e5b378f",
    );
    expect(videoIdFromInput("not a link")).toBeNull();
    expect(videoIdFromInput("")).toBeNull();
  });
});
