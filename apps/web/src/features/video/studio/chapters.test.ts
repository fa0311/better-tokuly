import { describe, expect, it } from "vitest";
import { type ChapterDraft, normalizeTag, toChapters, toDrafts, validateChapters, withChapterAt } from "./chapters";

const draft = (time: string, title = "part"): ChapterDraft => ({ key: time, time, title });

describe("chapters", () => {
  it("accepts chapters that start at zero and are far enough apart", () => {
    expect(validateChapters([draft("0:00"), draft("0:05"), draft("1:00")], 120)).toEqual([null, null, null]);
    expect(validateChapters([], 120)).toEqual([]);
  });

  it("says what is wrong with each row", () => {
    expect(validateChapters([draft("0:03")], 120)).toEqual(["first"]);
    expect(validateChapters([draft("0:00"), draft("soon")], 120)).toEqual([null, "time"]);
    expect(validateChapters([draft("0:00"), draft("0:04")], 120)).toEqual([null, "gap"]);
    expect(validateChapters([draft("0:00"), draft("1:00"), draft("0:30")], 120)).toEqual([null, null, "order"]);
    expect(validateChapters([draft("0:00"), draft("2:00")], 120)).toEqual([null, "end"]);
    expect(validateChapters([draft("0:00", "  ")], 120)).toEqual(["title"]);
  });

  it("does not judge the end while the length is unknown", () => {
    expect(validateChapters([draft("0:00"), draft("9:00")], 0)).toEqual([null, null]);
  });

  it("round-trips between the API and the form", () => {
    const drafts = toDrafts([
      { startSeconds: 0, title: "Intro" },
      { startSeconds: 75, title: " Main " },
    ]);
    expect(drafts.map((entry) => entry.time)).toEqual(["0:00", "1:15"]);
    expect(toChapters(drafts)).toEqual([
      { startSeconds: 0, title: "Intro" },
      { startSeconds: 75, title: "Main" },
    ]);
  });

  it("adds a chapter where it belongs in time, the first always at zero", () => {
    expect(withChapterAt([], 42).map((entry) => entry.time)).toEqual(["0:00"]);
    expect(withChapterAt([draft("0:00"), draft("1:00")], 30.7).map((entry) => entry.time)).toEqual(["0:00", "0:30", "1:00"]);
    expect(withChapterAt([draft("0:00")], 90).map((entry) => entry.time)).toEqual(["0:00", "1:30"]);
  });

  it("normalizes tags the way the server stores them", () => {
    expect(normalizeTag("  #Minecraft ")).toBe("minecraft");
    expect(normalizeTag("##ゲーム 実況")).toBe("ゲーム実況");
    expect(normalizeTag("#")).toBe("");
    expect(normalizeTag("x".repeat(40))).toHaveLength(30);
  });
});
