import { describe, expect, it } from "vitest";
import { parseWords } from "./words";

describe("the words of a synonym group", () => {
  it("are separated by commas, 読点 or line breaks", () => {
    expect(parseWords("スタンプ, ステッカー、sticker\nシール")).toEqual([
      "スタンプ",
      "ステッカー",
      "sticker",
      "シール",
    ]);
  });

  it("keep spaces inside a word, and drop blanks and repeats", () => {
    expect(parseWords(" ice cream ,, アイス\n\nIce Cream,アイス ")).toEqual([
      "ice cream",
      "アイス",
    ]);
    expect(parseWords("  \n , ")).toEqual([]);
  });
});
