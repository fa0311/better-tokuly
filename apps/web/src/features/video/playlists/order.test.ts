import { describe, expect, it } from "vitest";
import { movedItem } from "./order";

describe("movedItem", () => {
  const before = ["a", "b", "c", "d"];

  it("finds nothing when the order is unchanged", () => {
    expect(movedItem(before, ["a", "b", "c", "d"])).toBeNull();
  });

  it("recognises an item dragged down", () => {
    expect(movedItem(before, ["b", "c", "a", "d"])).toEqual({ id: "a", afterId: "c" });
    expect(movedItem(before, ["a", "c", "d", "b"])).toEqual({ id: "b", afterId: "d" });
  });

  it("recognises an item dragged up", () => {
    expect(movedItem(before, ["a", "d", "b", "c"])).toEqual({ id: "d", afterId: "a" });
  });

  it("uses an empty anchor for a move to the front", () => {
    expect(movedItem(before, ["c", "a", "b", "d"])).toEqual({ id: "c", afterId: "" });
  });

  it("treats a swap of neighbours as the first one moving down", () => {
    expect(movedItem(before, ["b", "a", "c", "d"])).toEqual({ id: "a", afterId: "b" });
  });
});
