import { describe, expect, it } from "vitest";
import { cellsForLevel, explorerLevel, formatPercent, levelUps, regionLevel } from "./levels";

describe("explorerLevel", () => {
  it("starts at level 1 and needs more cells for every level", () => {
    expect(explorerLevel(0)).toMatchObject({ level: 1, from: 0, next: 5, progress: 0, title: "beginner" });
    expect(explorerLevel(4).level).toBe(1);
    expect(explorerLevel(5)).toMatchObject({ level: 2, from: 5, next: 20, progress: 0 });
    expect(explorerLevel(20).level).toBe(3);
    expect(explorerLevel(12).progress).toBeCloseTo(7 / 15);
  });

  it("agrees with cellsForLevel at every boundary", () => {
    for (let level = 1; level < 60; level++) {
      expect(explorerLevel(cellsForLevel(level)).level).toBe(level);
      if (level > 1) expect(explorerLevel(cellsForLevel(level) - 1).level).toBe(level - 1);
    }
  });

  it("names the tiers", () => {
    expect(explorerLevel(cellsForLevel(6)).title).toBe("townie");
    expect(explorerLevel(cellsForLevel(40)).title).toBe("legend");
  });
});

describe("regionLevel", () => {
  it("measures progress between milestones", () => {
    expect(regionLevel(0)).toEqual({ level: 0, nextPercent: 0.001, progress: 0 });
    expect(regionLevel(0.0005)).toMatchObject({ level: 0, nextPercent: 0.001, progress: 0.5 });
    expect(regionLevel(0.03)).toMatchObject({ level: 2, nextPercent: 0.05 });
    expect(regionLevel(0.03).progress).toBeCloseTo(0.5);
    expect(regionLevel(100)).toEqual({ level: 15, nextPercent: 100, progress: 1 });
  });
});

describe("formatPercent", () => {
  it("keeps small shares readable", () => {
    expect(formatPercent(0)).toBe("0%");
    expect(formatPercent(0.00013012)).toBe("0.00013%");
    expect(formatPercent(0.0244)).toBe("0.024%");
    expect(formatPercent(0.5)).toBe("0.5%");
    expect(formatPercent(1.256)).toBe("1.26%");
    expect(formatPercent(12.34)).toBe("12.3%");
    expect(formatPercent(100)).toBe("100%");
    expect(formatPercent(3.6e-7)).toBe("0.00000036%");
  });
});

describe("levelUps", () => {
  it("records a first look without celebrating", () => {
    const ups = levelUps(null, 4, [{ id: "JP", level: 2 }]);
    expect(ups.explorer).toBeUndefined();
    expect(ups.regions).toEqual([]);
    expect(ups.seen).toEqual({ explorer: 4, regions: { JP: 2 } });
  });

  it("reports levels that rose since the last look", () => {
    const seen = { explorer: 2, regions: { JP: 1, "JP-13": 3 } };
    const ups = levelUps(seen, 3, [
      { id: "JP", level: 2 },
      { id: "JP-13", level: 3 },
      { id: "JP-14", level: 1 },
    ]);
    expect(ups.explorer).toBe(3);
    expect(ups.regions).toEqual([{ id: "JP", level: 2 }]);
    expect(ups.seen).toEqual({ explorer: 3, regions: { JP: 2, "JP-13": 3, "JP-14": 1 } });
  });

  it("stays quiet when nothing rose", () => {
    const seen = { explorer: 3, regions: { JP: 2 } };
    const ups = levelUps(seen, 3, [{ id: "JP", level: 2 }]);
    expect(ups.explorer).toBeUndefined();
    expect(ups.regions).toEqual([]);
  });
});
