import { wallet } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  checkoutStep,
  niceCeiling,
  parseCoins,
  payoutMoney,
  payoutProblem,
  safeCheckoutUrl,
} from "./format";

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
