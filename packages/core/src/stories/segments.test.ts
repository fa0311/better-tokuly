import { create, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { reelStart, storySegments } from "./segments";

const story = (id: string, items: number, viewed = false) =>
  create(notes.StorySchema, {
    id,
    viewed,
    items: Array.from({ length: items }, (_, index) => ({ caption: `${id}-${index}` })),
  });

describe("story segments", () => {
  it("plays every item of every story in order, each with its own key", () => {
    const segments = storySegments([story("a", 2), story("b", 1)]);
    expect(segments.map((segment) => segment.key)).toEqual(["a:0", "a:1", "b:0"]);
    expect(segments[1]?.item.caption).toBe("a-1");
  });

  it("starts at the first story not watched yet", () => {
    expect(reelStart([story("a", 2, true), story("b", 1), story("c", 1)])).toBe(2);
  });

  it("starts at the story asked for, watched or not", () => {
    expect(reelStart([story("a", 1), story("b", 1, true)], "b")).toBe(1);
  });

  it("starts at the beginning when everything was watched, or the story is gone", () => {
    expect(reelStart([story("a", 1, true), story("b", 1, true)])).toBe(0);
    expect(reelStart([story("a", 1)], "gone")).toBe(0);
  });
});
