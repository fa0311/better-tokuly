import { describe, expect, it } from "vitest";
import { parsePendingAsk } from "./guest-return";
import { countdownClock, localized, parseMutedWords, wrapText } from "./model";
import { noteServerTime, serverNow } from "./server-clock";

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
    expect(parseMutedWords("きらい, うざい、きらい\n ばか ，")).toEqual([
      "きらい",
      "うざい",
      "ばか",
    ]);
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
    expect(
      parsePendingAsk(stored({ handle: "alice", text: "好きな色は?", at: now - 60_000 }), now),
    ).toEqual({ handle: "alice", text: "好きな色は?" });
  });
  it("forgets an entry after a quarter of an hour, and one from the future", () => {
    expect(
      parsePendingAsk(stored({ handle: "alice", text: "まだ?", at: now - 14 * 60_000 }), now),
    ).not.toBeNull();
    expect(
      parsePendingAsk(stored({ handle: "alice", text: "まだ?", at: now - 16 * 60_000 }), now),
    ).toBeNull();
    expect(
      parsePendingAsk(stored({ handle: "alice", text: "まだ?", at: now + 1000 }), now),
    ).toBeNull();
  });
  it("has nothing to bring back when nothing was written", () => {
    expect(parsePendingAsk(stored({ handle: "alice", text: "", at: now }), now)).toBeNull();
    expect(parsePendingAsk(stored({ handle: "alice", text: "  \n", at: now }), now)).toBeNull();
    expect(parsePendingAsk(stored({ handle: "alice", at: now }), now)).toBeNull();
  });
  it("refuses anything that is not a handle (the value becomes part of an address)", () => {
    expect(parsePendingAsk(stored({ handle: "../admin", text: "x", at: now }), now)).toBeNull();
    expect(parsePendingAsk(stored({ handle: 42, text: "x", at: now }), now)).toBeNull();
    expect(parsePendingAsk("not json", now)).toBeNull();
    expect(parsePendingAsk(null, now)).toBeNull();
  });
});

describe("the server clock", () => {
  it("is the device clock until the server said what time it is", () => {
    expect(serverNow(5000)).toBe(5000);
  });
  it("follows the server when the device is ahead or behind", () => {
    noteServerTime(1_000_000, 1_600_000); // the device is ten minutes ahead
    expect(serverNow(1_660_000)).toBe(1_060_000);
    noteServerTime(2_000_000, 1_000_000); // the device is behind
    expect(serverNow(1_000_500)).toBe(2_000_500);
  });
  it("ignores a frame without a time", () => {
    noteServerTime(3000, 3000);
    noteServerTime(0, 9000);
    expect(serverNow(4000)).toBe(4000);
  });
});
