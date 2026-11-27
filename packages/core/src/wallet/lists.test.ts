import { describe, expect, it } from "vitest";
import { daysAgo, groupByDay, uniqueById } from "./lists";

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

describe("uniqueById", () => {
  it("keeps the first of each ID, in order", () => {
    expect(uniqueById([{ id: "a" }, { id: "b" }, { id: "a" }])).toEqual([{ id: "a" }, { id: "b" }]);
  });
});
