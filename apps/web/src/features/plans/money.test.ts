import { describe, expect, it } from "vitest";
import { formatMoney, minorDigits, moneyInputValue, parseMoney } from "./money";

describe("money", () => {
  it("knows how many decimals a currency has", () => {
    expect(minorDigits("JPY")).toBe(0);
    expect(minorDigits("USD")).toBe(2);
  });

  it("formats minor units", () => {
    expect(formatMoney(1200n, "JPY", "ja")).toBe("1,200円");
    expect(formatMoney(1200n, "JPY", "en")).toBe("¥1,200");
    expect(formatMoney(1250n, "USD", "en")).toBe("$12.50");
  });

  it("reads what people type", () => {
    expect(parseMoney("1,200", "JPY")).toBe(1200n);
    expect(parseMoney("１２００", "JPY")).toBe(1200n);
    expect(parseMoney("12.5", "USD")).toBe(1250n);
    expect(parseMoney("12", "USD")).toBe(1200n);
  });

  it("refuses what is not a positive amount of that currency", () => {
    expect(parseMoney("", "JPY")).toBeNull();
    expect(parseMoney("0", "JPY")).toBeNull();
    expect(parseMoney("-5", "JPY")).toBeNull();
    expect(parseMoney("12.5", "JPY")).toBeNull();
    expect(parseMoney("1.234", "USD")).toBeNull();
    expect(parseMoney("abc", "USD")).toBeNull();
  });

  it("writes an amount back into a field", () => {
    expect(moneyInputValue(1200n, "JPY")).toBe("1200");
    expect(moneyInputValue(1250n, "USD")).toBe("12.50");
    expect(moneyInputValue(5n, "USD")).toBe("0.05");
  });
});
