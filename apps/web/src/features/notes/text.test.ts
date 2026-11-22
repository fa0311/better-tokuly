import { describe, expect, it } from "vitest";
import { applyCompletion, charCount, completionAt, normalizeHashtag, parseNoteText } from "./text";

const kinds = (text: string) => parseNoteText(text).map((segment) => `${segment.type}:${segment.text}`);

describe("charCount", () => {
  it("counts what people see", () => {
    expect(charCount("今何してる?")).toBe(6);
    expect(charCount("👨‍👩‍👧‍👦🇯🇵")).toBe(2);
    expect(charCount("")).toBe(0);
  });
});

describe("parseNoteText", () => {
  it("finds hashtags, mentions and links in order", () => {
    expect(kinds("やっほー #Tokuly @taro_01 https://example.com/a?b=1 またね")).toEqual([
      "text:やっほー ",
      "hashtag:#Tokuly",
      "text: ",
      "mention:@taro_01",
      "text: ",
      "url:https://example.com/a?b=1",
      "text: またね",
    ]);
  });

  it("normalizes hashtags like the server", () => {
    const [segment] = parseNoteText("＃ＴＯＫＵＬＹ");
    expect(segment).toEqual({ type: "hashtag", text: "＃ＴＯＫＵＬＹ", tag: "tokuly" });
    expect(normalizeHashtag("#123")).toBeUndefined();
    expect(normalizeHashtag("#渋谷_2026")).toBe("渋谷_2026");
  });

  it("leaves a # or @ glued to a word alone", () => {
    expect(kinds("C#が好き")).toEqual(["text:C#が好き"]);
    expect(kinds("mail@example.com")).toEqual(["text:mail@example.com"]);
    expect(kinds("色#fff")).toEqual(["text:色#fff"]);
  });

  it("mentions follow Japanese text without a space", () => {
    expect(kinds("友達の@taro")).toEqual(["text:友達の", "mention:@taro"]);
  });

  it("does not read hashtags or mentions inside links", () => {
    expect(kinds("https://example.com/#top https://example.com/@someone")).toEqual(["url:https://example.com/#top", "text: ", "url:https://example.com/@someone"]);
  });

  it("ends a link at Japanese text and before sentence punctuation", () => {
    expect(kinds("見てhttps://example.com/aだよ")).toEqual(["text:見て", "url:https://example.com/a", "text:だよ"]);
    expect(kinds("(https://example.com/a).")).toEqual(["text:(", "url:https://example.com/a", "text:)."]);
    expect(kinds("https://en.wikipedia.org/wiki/Tokyo_(disambiguation)")).toEqual(["url:https://en.wikipedia.org/wiki/Tokyo_(disambiguation)"]);
  });

  it("treats addresses on other services and short handles as text", () => {
    expect(kinds("@name@example.com")).toEqual(["text:@name@example.com"]);
    expect(kinds("@ab")).toEqual(["text:@ab"]);
  });
});

describe("completion", () => {
  it("offers the hashtag or mention being typed at the caret", () => {
    expect(completionAt("今日は #しぶ", 7)).toEqual({ kind: "hashtag", query: "しぶ", start: 4, end: 7 });
    expect(completionAt("@ta", 3)).toEqual({ kind: "mention", query: "ta", start: 0, end: 3 });
    expect(completionAt("友達の@ta", 6)).toEqual({ kind: "mention", query: "ta", start: 3, end: 6 });
    expect(completionAt("#", 1)).toEqual({ kind: "hashtag", query: "", start: 0, end: 1 });
  });

  it("offers nothing in the middle of a word or an address", () => {
    expect(completionAt("C#", 2)).toBeUndefined();
    expect(completionAt("mail@ex", 7)).toBeUndefined();
    expect(completionAt("ふつうの文", 5)).toBeUndefined();
  });

  it("replaces the token and moves the caret behind it", () => {
    const text = "今日は #しぶ だよ";
    const completion = completionAt(text, 7)!;
    expect(applyCompletion(text, completion, "#渋谷")).toEqual({ text: "今日は #渋谷 だよ", caret: 8 });
  });
});
