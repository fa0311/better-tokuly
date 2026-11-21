import { describe, expect, it } from "vitest";
import { netEarnings } from "./pack-form";
import { niceCeiling } from "./pack-stats";

describe("chart axis", () => {
  it("ends on a clean number at or above the peak", () => {
    expect(niceCeiling(0)).toBe(1);
    expect(niceCeiling(1)).toBe(1);
    expect(niceCeiling(3)).toBe(5);
    expect(niceCeiling(17)).toBe(20);
    expect(niceCeiling(50)).toBe(50);
    expect(niceCeiling(51)).toBe(100);
    expect(niceCeiling(1234)).toBe(2000);
  });
});

describe("what a creator keeps of a sale", () => {
  it("takes the platform's share and rounds down to whole coins", () => {
    expect(netEarnings(100, 30)).toBe(70);
    expect(netEarnings(15, 30)).toBe(10);
    expect(netEarnings(10, 0)).toBe(10);
  });
});
