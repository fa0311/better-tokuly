import { describe, expect, it } from "vitest";
import { collectedSteps, countdown, formatMoney, parseMemo } from "./format";

describe("formatMoney", () => {
  it("writes minor units in the currency's own precision", () => {
    expect(formatMoney(1000n, "JPY", "en")).toBe("¥1,000");
    expect(formatMoney(499n, "USD", "en")).toBe("$4.99");
  });

  it("falls back to the code for something that is not a currency", () => {
    expect(formatMoney(1200n, "coins!", "en")).toBe("12.00 coins!");
  });
});

describe("parseMemo", () => {
  it("keeps scalar values as text", () => {
    expect(parseMemo('{"sku":"coins_500","bonus_coins":20,"gift":true,"nested":{"a":1}}')).toEqual({
      sku: "coins_500",
      bonus_coins: "20",
      gift: "true",
    });
  });

  it("is empty for missing or broken memos", () => {
    expect(parseMemo("")).toEqual({});
    expect(parseMemo("{oops")).toEqual({});
    expect(parseMemo("[1,2]")).toEqual({});
  });
});

describe("countdown", () => {
  it("rounds up to the minute and never goes negative", () => {
    expect(countdown(3 * 3_600_000 + 61_000, 0)).toEqual({ hours: 3, minutes: 2 });
    expect(countdown(20_000, 0)).toEqual({ hours: 0, minutes: 1 });
    expect(countdown(0, 5_000)).toEqual({ hours: 0, minutes: 0 });
  });
});

describe("collectedSteps", () => {
  const cycle = [10n, 10n, 15n, 15n, 20n, 20n, 50n];

  it("counts the steps before the next one", () => {
    expect(collectedSteps({ claimable: true, streak: 0, nextCycleDay: 1, cycle })).toBe(0);
    expect(collectedSteps({ claimable: true, streak: 2, nextCycleDay: 3, cycle })).toBe(2);
    expect(collectedSteps({ claimable: false, streak: 3, nextCycleDay: 4, cycle })).toBe(3);
  });

  it("shows a finished round as complete until the next day", () => {
    expect(collectedSteps({ claimable: false, streak: 7, nextCycleDay: 1, cycle })).toBe(7);
    expect(collectedSteps({ claimable: true, streak: 7, nextCycleDay: 1, cycle })).toBe(0);
  });
});
