import { describe, expect, it } from "vitest";
import { dayLabel, fileSize, listTime, mentionAt, playTime, tokenize } from "./text";

describe("message text", () => {
  it("makes links tappable and keeps the sentence's punctuation out of them", () => {
    expect(tokenize("見て https://tokuly.example/notes/1。すごい", [], false)).toEqual([
      { type: "text", text: "見て " },
      {
        type: "link",
        text: "https://tokuly.example/notes/1",
        href: "https://tokuly.example/notes/1",
      },
      { type: "text", text: "。すごい" },
    ]);
    expect(tokenize("(https://a.example/x?y=1)", [], false)[1]).toMatchObject({
      type: "link",
      href: "https://a.example/x?y=1",
    });
    expect(tokenize("https://a.example/wiki/Tokyo_(city) だよ", [], false)[0]).toMatchObject({
      href: "https://a.example/wiki/Tokyo_(city)",
    });
    expect(
      tokenize("ここhttps://a.example/aを見て！", [], false).map((token) => token.text),
    ).toEqual(["ここ", "https://a.example/a", "を見て！"]);
  });

  it("marks only the mentions the server recognised", () => {
    const tokens = tokenize("@aoi と @stranger で @Aoi", ["aoi"], false);
    expect(tokens.filter((token) => token.type === "mention").map((token) => token.text)).toEqual([
      "@aoi",
      "@Aoi",
    ]);
    expect(tokens.some((token) => token.type === "text" && token.text.includes("@stranger"))).toBe(
      true,
    );
  });

  it("marks @all only when it reached everyone", () => {
    expect(tokenize("@all 集合！", [], true)[0]).toEqual({ type: "everyone", text: "@all" });
    expect(tokenize("@all 集合！", [], false)[0]).toEqual({ type: "text", text: "@all 集合！" });
  });

  it("does not take an e-mail address for a mention", () => {
    expect(tokenize("mail@aoi", ["aoi"], false)).toEqual([{ type: "text", text: "mail@aoi" }]);
  });

  it("finds the mention being typed at the caret", () => {
    expect(mentionAt("こんにちは @ao", 9)).toEqual({ query: "ao", start: 6 });
    expect(mentionAt("こんにちは @ao です", 9)).toEqual({ query: "ao", start: 6 });
    expect(mentionAt("@", 1)).toEqual({ query: "", start: 0 });
    expect(mentionAt("mail@ao", 7)).toBeNull();
    expect(mentionAt("@aoi おはよ", 8)).toBeNull();
  });
});

describe("times and sizes", () => {
  const now = new Date(2026, 10, 3, 15, 0, 0).getTime();
  const at = (year: number, month: number, day: number, hour = 9, minute = 5) =>
    new Date(year, month - 1, day, hour, minute).getTime();

  it("labels days the way people say them", () => {
    expect(dayLabel(at(2026, 11, 3), now)).toBe("今日");
    expect(dayLabel(at(2026, 11, 2, 23, 59), now)).toBe("昨日");
    expect(dayLabel(at(2026, 10, 20), now)).toBe("10月20日(火)");
    expect(dayLabel(at(2025, 12, 31), now)).toBe("2025年12月31日(水)");
  });

  it("shortens the time of the last activity as it gets older", () => {
    expect(listTime(at(2026, 11, 3), now)).toBe("09:05");
    expect(listTime(at(2026, 11, 2), now)).toBe("昨日");
    expect(listTime(at(2026, 10, 30), now)).toBe("金曜日");
    expect(listTime(at(2026, 9, 1), now)).toBe("9/1");
    expect(listTime(at(2025, 9, 1), now)).toBe("2025/9/1");
    expect(listTime(0, now)).toBe("");
  });

  it("formats lengths of time like a player", () => {
    expect(playTime(7)).toBe("0:07");
    expect(playTime(205)).toBe("3:25");
    expect(playTime(3723)).toBe("1:02:03");
    expect(playTime(-4)).toBe("0:00");
  });

  it("formats file sizes", () => {
    expect(fileSize(820)).toBe("820 B");
    expect(fileSize(12_400)).toBe("12 KB");
    expect(fileSize(3_400_000)).toBe("3.4 MB");
    expect(fileSize(1_200_000_000)).toBe("1.2 GB");
  });
});
