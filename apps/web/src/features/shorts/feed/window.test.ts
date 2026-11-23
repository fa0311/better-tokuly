import { describe, expect, it } from "vitest";
import { moveWindow } from "./window";

const pages = (...entries: [string, number][]) =>
  entries.map(([param, length]) => ({ param, length }));

describe("moveWindow", () => {
  const start = { pages: pages(["", 10], ["a", 10], ["b", 10]), offset: 0 };

  it("keeps the window when nothing moved", () => {
    expect(moveWindow(start, pages(["", 10], ["a", 10], ["b", 10]))).toBe(start);
  });

  it("keeps the offset while pages are added at the end or shrink", () => {
    expect(moveWindow(start, pages(["", 9], ["a", 10], ["b", 10], ["c", 10])).offset).toBe(0);
  });

  it("moves on by what the pages let go at the front held", () => {
    const moved = moveWindow(start, pages(["b", 10], ["c", 10]));
    expect(moved.offset).toBe(20);
    // And back by what the pages read again hold.
    expect(moveWindow(moved, pages(["a", 10], ["b", 10], ["c", 10])).offset).toBe(10);
  });

  it("starts over for a feed that was reset", () => {
    expect(moveWindow({ ...start, offset: 30 }, pages(["x", 10])).offset).toBe(0);
    expect(moveWindow({ ...start, offset: 30 }, []).offset).toBe(0);
  });
});
