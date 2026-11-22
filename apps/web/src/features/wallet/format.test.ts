import { wallet } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  checkoutStep,
  collectedSteps,
  countdown,
  daysAgo,
  formatMoney,
  groupByDay,
  niceCeiling,
  parseCoins,
  parseMemo,
  payoutMoney,
  payoutProblem,
  safeCheckoutUrl,
} from "./format";

describe("formatMoney", () => {
  it("writes minor units in the currency's own precision", () => {
    expect(formatMoney(1000n, "JPY", "en")).toBe("¥1,000");
    expect(formatMoney(499n, "USD", "en")).toBe("$4.99");
  });

  it("falls back to the code for something that is not a currency", () => {
    expect(formatMoney(1200n, "coins!", "en")).toBe("12.00 coins!");
  });
});

describe("payoutMoney", () => {
  it("applies the rate per 100 coins and rounds down", () => {
    expect(payoutMoney(1200n, 100n)).toBe(1200n);
    expect(payoutMoney(1250n, 50n)).toBe(625n);
    expect(payoutMoney(199n, 1n)).toBe(1n);
  });
});

describe("parseCoins", () => {
  it("reads plain, separated and full-width digits", () => {
    expect(parseCoins("1200")).toBe(1200n);
    expect(parseCoins(" 1,200 ")).toBe(1200n);
    expect(parseCoins("１２００")).toBe(1200n);
  });

  it("rejects anything that is not a whole number", () => {
    for (const text of ["", "12.5", "-3", "abc", "1e3"]) expect(parseCoins(text)).toBeUndefined();
  });
});

describe("payoutProblem", () => {
  it("accepts amounts between the minimum and what is available", () => {
    expect(payoutProblem(1000n, 2500n, 1000n)).toBeNull();
    expect(payoutProblem(2500n, 2500n, 1000n)).toBeNull();
  });

  it("names what is wrong otherwise", () => {
    expect(payoutProblem(999n, 2500n, 1000n)).toBe("belowMinimum");
    expect(payoutProblem(0n, 2500n, 0n)).toBe("belowMinimum");
    expect(payoutProblem(2501n, 2500n, 1000n)).toBe("aboveAvailable");
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

describe("groupByDay", () => {
  it("keeps the order and starts a group whenever the local day changes", () => {
    const at = (day: number, hour: number) => new Date(2026, 10, day, hour);
    const groups = groupByDay([at(22, 9), at(22, 1), at(21, 23), at(20, 12)], (date) => date);
    expect(groups.map((group) => group.items.length)).toEqual([2, 1, 1]);
  });
});

describe("daysAgo", () => {
  it("counts calendar days, not 24-hour spans", () => {
    const now = new Date(2026, 10, 22, 0, 30);
    expect(daysAgo(new Date(2026, 10, 22, 0, 5), now)).toBe(0);
    expect(daysAgo(new Date(2026, 10, 21, 23, 55), now)).toBe(1);
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

describe("checkoutStep", () => {
  it("sends a pending purchase to the provider's page when there is one", () => {
    expect(
      checkoutStep({
        status: wallet.PurchaseStatus.PENDING,
        checkoutUrl: "https://pay.example/c/1",
      }),
    ).toBe("redirect");
    expect(checkoutStep({ status: wallet.PurchaseStatus.PENDING, checkoutUrl: "" })).toBe(
      "waiting",
    );
  });

  it("maps the settled states", () => {
    expect(checkoutStep({ status: wallet.PurchaseStatus.COMPLETED, checkoutUrl: "" })).toBe(
      "completed",
    );
    expect(checkoutStep({ status: wallet.PurchaseStatus.FAILED, checkoutUrl: "" })).toBe("failed");
    expect(checkoutStep({ status: wallet.PurchaseStatus.CANCELLED, checkoutUrl: "" })).toBe(
      "cancelled",
    );
    expect(checkoutStep({ status: wallet.PurchaseStatus.EXPIRED, checkoutUrl: "" })).toBe(
      "expired",
    );
  });
});

describe("safeCheckoutUrl", () => {
  it("lets web addresses through and nothing else", () => {
    expect(safeCheckoutUrl("https://pay.example/c/1")).toBe("https://pay.example/c/1");
    expect(safeCheckoutUrl("javascript:alert(1)")).toBeUndefined();
    expect(safeCheckoutUrl("not a url")).toBeUndefined();
  });
});

describe("niceCeiling", () => {
  it("rounds up to a number that halves cleanly", () => {
    expect(niceCeiling(0)).toBe(2);
    expect(niceCeiling(3)).toBe(4);
    expect(niceCeiling(7)).toBe(10);
    expect(niceCeiling(120)).toBe(200);
    expect(niceCeiling(1000)).toBe(1000);
    expect(niceCeiling(1300)).toBe(2000);
    expect(niceCeiling(4100)).toBe(5000);
    expect(niceCeiling(87_000)).toBe(100_000);
  });
});
