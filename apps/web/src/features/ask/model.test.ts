import { describe, expect, it } from "vitest";
import { parsePendingAsk } from "./guest-return";
import { answerShareUrl, boxShareUrl, charCount, countdownClock, localized, parseMutedWords, wrapText } from "./model";

describe("charCount", () => {
  it("counts characters the way the server does, not UTF-16 units", () => {
    expect(charCount("質問箱")).toBe(3);
    expect(charCount("🔥🔥")).toBe(2);
    expect(charCount("")).toBe(0);
  });
});

describe("localized", () => {
  const text = { ja: "一番おしゃれな人は?", en: "Who has the best style?" } as never;
  it("picks the language the app is shown in", () => {
    expect(localized(text, "ja")).toBe("一番おしゃれな人は?");
    expect(localized(text, "en-US")).toBe("Who has the best style?");
  });
  it("falls back to the other language, and to nothing", () => {
    expect(localized({ ja: "", en: "Only English" } as never, "ja")).toBe("Only English");
    expect(localized(undefined, "ja")).toBe("");
  });
});

describe("share links", () => {
  it("point at the public page of the box", () => {
    expect(boxShareUrl("alice", "https://tokuly.example")).toBe("https://tokuly.example/q/alice");
    expect(answerShareUrl("alice", "q1", "https://tokuly.example")).toBe("https://tokuly.example/q/alice?a=q1");
  });
});

describe("countdownClock", () => {
  it("shows minutes and seconds, then hours", () => {
    expect(countdownClock(59_000)).toBe("00:59");
    expect(countdownClock(3_599_000)).toBe("59:59");
    expect(countdownClock(3_723_000)).toBe("1:02:03");
  });
  it("rounds a started second up and never goes below zero", () => {
    expect(countdownClock(1)).toBe("00:01");
    expect(countdownClock(-5000)).toBe("00:00");
  });
});

describe("parseMutedWords", () => {
  it("splits on commas of either width and line breaks, dropping blanks and repeats", () => {
    expect(parseMutedWords("きらい, うざい、きらい\n ばか ，")).toEqual(["きらい", "うざい", "ばか"]);
    expect(parseMutedWords("  ")).toEqual([]);
  });
});

describe("wrapText", () => {
  // Every character is one unit wide.
  const measure = (text: string) => Array.from(text).length;

  it("breaks Japanese between characters", () => {
    expect(wrapText("なんでも質問してね", 4, 9, measure)).toEqual(["なんでも", "質問して", "ね"]);
  });
  it("keeps Latin words whole where they fit", () => {
    expect(wrapText("ask me anything", 8, 9, measure)).toEqual(["ask me", "anything"]);
  });
  it("cuts a word that is wider than a line", () => {
    expect(wrapText("abcdefghij", 4, 9, measure)).toEqual(["abcd", "efgh", "ij"]);
  });
  it("keeps line breaks of the text", () => {
    expect(wrapText("一行目\n二行目", 10, 9, measure)).toEqual(["一行目", "二行目"]);
  });
  it("ends the last allowed line in an ellipsis when text is left over", () => {
    expect(wrapText("あいうえおかきくけこ", 4, 2, measure)).toEqual(["あいうえ", "おかき…"]);
    expect(wrapText("あいうえ", 4, 2, measure)).toEqual(["あいうえ"]);
  });
});

describe("parsePendingAsk", () => {
  const now = 1_800_000_000_000;
  const stored = (value: object) => JSON.stringify(value);

  it("reads a fresh entry", () => {
    expect(parsePendingAsk(stored({ handle: "alice", text: "好きな色は?", at: now - 60_000 }), now)).toEqual({ handle: "alice", text: "好きな色は?" });
  });
  it("forgets an entry after an hour, and one from the future", () => {
    expect(parsePendingAsk(stored({ handle: "alice", text: "", at: now - 61 * 60_000 }), now)).toBeNull();
    expect(parsePendingAsk(stored({ handle: "alice", text: "", at: now + 1000 }), now)).toBeNull();
  });
  it("refuses anything that is not a handle (the value becomes part of an address)", () => {
    expect(parsePendingAsk(stored({ handle: "../admin", text: "", at: now }), now)).toBeNull();
    expect(parsePendingAsk(stored({ handle: 42, at: now }), now)).toBeNull();
    expect(parsePendingAsk("not json", now)).toBeNull();
    expect(parsePendingAsk(null, now)).toBeNull();
  });
});
