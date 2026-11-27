import { create, wallet } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  fitsLimit,
  limitPresets,
  limitSummary,
  ownLimitProblem,
  packageYen,
  resetDay,
  usedShare,
} from "./limit";

const minor = create(wallet.SpendingLimitSchema, {
  ceilingYen: 5000n,
  limitYen: 5000n,
  spentYen: 3800n,
  remainingYen: 1200n,
});
const adult = create(wallet.SpendingLimitSchema, { spentYen: 12_000n });

describe("fitsLimit", () => {
  it("compares with what is left this month", () => {
    expect(fitsLimit(minor, 1200n)).toBe(true);
    expect(fitsLimit(minor, 1201n)).toBe(false);
  });

  it("lets everything through without a limit or before it is known", () => {
    expect(fitsLimit(adult, 1_000_000n)).toBe(true);
    expect(fitsLimit(undefined, 1n)).toBe(true);
  });
});

describe("packageYen", () => {
  it("counts the price in yen, otherwise the coins", () => {
    expect(packageYen({ priceMinor: 1000n, currency: "JPY", coins: 1000n })).toBe(1000n);
    expect(packageYen({ priceMinor: 499n, currency: "USD", coins: 500n })).toBe(500n);
  });
});

describe("the own limit", () => {
  it("offers minors no choice above the ceiling", () => {
    expect(limitPresets(minor)).toEqual([0n, 1000n, 3000n, 5000n]);
    expect(limitPresets(adult)).toContain(30_000n);
  });

  it("refuses what the server would refuse", () => {
    expect(ownLimitProblem(minor, 5001n)).toBe("SPENDING_LIMIT_RESTRICTED");
    expect(ownLimitProblem(minor, null)).toBe("SPENDING_LIMIT_RESTRICTED");
    expect(ownLimitProblem(minor, 0n)).toBeNull();
    expect(ownLimitProblem(adult, null)).toBeNull();
    expect(ownLimitProblem(adult, -1n)).toBe("SPENDING_LIMIT_INVALID");
  });

  it("measures the month against the limit", () => {
    expect(usedShare(minor)).toBeCloseTo(0.76);
    expect(usedShare(adult)).toBeNull();
    expect(usedShare({ limitYen: 0n, spentYen: 0n })).toBe(1);
  });
});

describe("resetDay", () => {
  it("names the first of the month in Japan time", () => {
    // 2026-11-30T15:00Z is midnight of December 1st in Tokyo.
    const limit = create(wallet.SpendingLimitSchema, { resetsAt: { seconds: 1_796_050_800n } });
    expect(resetDay(limit, "ja")).toBe("12月1日");
    expect(resetDay(limit, "en")).toBe("Dec 1");
  });
});

describe("limitSummary", () => {
  it("says what is left, that it is used up, or that there is no limit", () => {
    expect(limitSummary(minor, "ja")).toEqual({
      key: "limit.remaining",
      values: { amount: "￥1,200" },
    });
    expect(
      limitSummary(create(wallet.SpendingLimitSchema, { limitYen: 0n, remainingYen: 0n }), "en")
        .key,
    ).toBe("limit.reached");
    expect(limitSummary(adult, "en").key).toBe("limit.none");
  });
});
