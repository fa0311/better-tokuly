import { describe, expect, it } from "vitest";
import { nameFromFile, parseKeywords } from "./stamp-meta";

describe("stamp search words", () => {
  it("splits on spaces and commas, Japanese ones included", () => {
    expect(parseKeywords("ねこ, わーい  cat、えがお，smile")).toEqual(["ねこ", "わーい", "cat", "えがお", "smile"]);
  });

  it("drops '#', lowercases and removes repeats", () => {
    expect(parseKeywords("#Cat cat CAT #dog")).toEqual(["cat", "dog"]);
  });

  it("keeps to the API's limits", () => {
    const many = Array.from({ length: 14 }, (_, index) => `w${index}`).join(" ");
    expect(parseKeywords(many)).toHaveLength(10);
    expect(parseKeywords("あ".repeat(40))).toEqual(["あ".repeat(24)]);
    expect(parseKeywords("   ")).toEqual([]);
  });
});

describe("a stamp name from a file name", () => {
  it("drops the extension and separators", () => {
    expect(nameFromFile("happy_cat-01.png")).toBe("happy cat 01");
    expect(nameFromFile("わーい.webp")).toBe("わーい");
  });

  it("fits the name limit", () => {
    expect(Array.from(nameFromFile(`${"😀".repeat(60)}.png`))).toHaveLength(40);
  });
});
