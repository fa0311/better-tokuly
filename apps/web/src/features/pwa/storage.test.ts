import { describe, expect, it } from "vitest";
import { formatBytes, usedPercent } from "./storage";

describe("formatBytes", () => {
  it("picks the unit people would use", () => {
    expect(formatBytes(0, "en")).toBe("0 kB");
    expect(formatBytes(12, "en")).toBe("1 kB");
    expect(formatBytes(1500, "en")).toBe("2 kB");
    expect(formatBytes(12_340_000, "en")).toBe("12.3 MB");
    expect(formatBytes(2_100_000_000, "en")).toBe("2.1 GB");
  });

  it("drops the decimal when it says nothing", () => {
    expect(formatBytes(120_400_000, "en")).toBe("120 MB");
    expect(formatBytes(3_000_000, "en")).toBe("3 MB");
  });

  it("stops at gigabytes", () => {
    expect(formatBytes(5_000_000_000_000, "en")).toBe("5,000 GB");
  });

  it("formats numbers for the language", () => {
    expect(formatBytes(12_340_000, "ja")).toBe("12.3 MB");
  });
});

describe("usedPercent", () => {
  it("is the share of the quota", () => {
    expect(usedPercent({ usedBytes: 250, quotaBytes: 1000 })).toBe(25);
  });

  it("shows a sliver for a little, nothing for nothing, and never more than all", () => {
    expect(usedPercent({ usedBytes: 1, quotaBytes: 1_000_000 })).toBe(1);
    expect(usedPercent({ usedBytes: 0, quotaBytes: 1000 })).toBe(0);
    expect(usedPercent({ usedBytes: 2000, quotaBytes: 1000 })).toBe(100);
  });
});
