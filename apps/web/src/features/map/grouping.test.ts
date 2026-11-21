import { describe, expect, it } from "vitest";
import { groupByProximity } from "./grouping";

// ~11 m per 0.0001° of latitude.
const at = (id: string, lat: number, lng = 139.7) => ({ id, lat, lng });

describe("groupByProximity", () => {
  it("keeps distant points apart", () => {
    const groups = groupByProximity([at("a", 35.0), at("b", 35.01)], 30);
    expect(groups.map((g) => g.members.map((m) => m.id))).toEqual([["a"], ["b"]]);
  });

  it("joins points within the radius and centres the group", () => {
    const groups = groupByProximity([at("a", 35.0), at("b", 35.0002)], 30);
    expect(groups).toHaveLength(1);
    expect(groups[0]!.members.map((m) => m.id).sort()).toEqual(["a", "b"]);
    expect(groups[0]!.lat).toBeCloseTo(35.0001, 6);
  });

  it("chains neighbours transitively", () => {
    // a–b and b–c are ~22 m apart; a–c is ~44 m (outside the radius on its own).
    const groups = groupByProximity([at("a", 35.0), at("b", 35.0002), at("c", 35.0004), at("far", 35.1)], 30);
    const sizes = groups.map((g) => g.members.length).sort();
    expect(sizes).toEqual([1, 3]);
  });

  it("handles empty input", () => {
    expect(groupByProximity([], 30)).toEqual([]);
  });
});
