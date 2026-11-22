import { describe, expect, it } from "vitest";
import { completeMention, mentionAtCaret, tokenize } from "./text";

describe("comment text", () => {
  const handles = new Set(["yuki", "taro_99"]);

  it("links mentions of users the server resolved, and only those", () => {
    expect(tokenize("やっほー @Yuki と @nobody !", handles, false)).toEqual([
      { kind: "text", text: "やっほー " },
      { kind: "mention", text: "@Yuki", handle: "yuki" },
      { kind: "text", text: " と @nobody !" },
    ]);
  });

  it("does not take an e-mail address for a mention", () => {
    expect(tokenize("mail yuki@yuki.example", handles, false)).toEqual([
      { kind: "text", text: "mail yuki@yuki.example" },
    ]);
  });

  it("links web addresses without the sentence's punctuation", () => {
    expect(tokenize("見て！https://example.com/a?b=1。すごい", handles, false)).toEqual([
      { kind: "text", text: "見て！" },
      { kind: "link", text: "https://example.com/a?b=1", url: "https://example.com/a?b=1" },
      { kind: "text", text: "。すごい" },
    ]);
  });

  it("makes clock times jump targets on videos only", () => {
    expect(tokenize("1:23のとこ好き、あと1:02:03も", handles, true)).toEqual([
      { kind: "time", text: "1:23", seconds: 83 },
      { kind: "text", text: "のとこ好き、あと" },
      { kind: "time", text: "1:02:03", seconds: 3723 },
      { kind: "text", text: "も" },
    ]);
    expect(tokenize("1:23のとこ", handles, false)).toEqual([{ kind: "text", text: "1:23のとこ" }]);
    // Not a time: the seconds run past 59.
    expect(tokenize("比率は 3:75", handles, true)).toEqual([{ kind: "text", text: "比率は 3:75" }]);
  });

  it("finds the mention being typed at the caret", () => {
    expect(mentionAtCaret("hi @yu", 6)).toEqual({ start: 3, query: "yu" });
    expect(mentionAtCaret("@", 1)).toEqual({ start: 0, query: "" });
    expect(mentionAtCaret("こんにちは@ta", 8)).toEqual({ start: 5, query: "ta" });
    expect(mentionAtCaret("hi @yu ki", 9)).toBeNull();
    expect(mentionAtCaret("mail a@b", 8)).toBeNull();
    expect(mentionAtCaret("hi @yu", 2)).toBeNull();
  });

  it("completes a mention and puts the caret after it", () => {
    expect(completeMention("hi @yu there", 3, 6, "yuki")).toEqual({
      text: "hi @yuki there",
      caret: 9,
    });
    expect(completeMention("@ta", 0, 3, "taro_99")).toEqual({ text: "@taro_99 ", caret: 9 });
  });
});
