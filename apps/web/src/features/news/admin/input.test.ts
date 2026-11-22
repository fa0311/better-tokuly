import { describe, expect, it } from "vitest";
import { articleIdFrom, parseTags } from "./input";

describe("what staff type", () => {
  it("default tags: commas, 読点 or spaces, '#' optional, no repeats", () => {
    expect(parseTags("tech, #ゲーム、 Tech　音楽")).toEqual(["tech", "ゲーム", "音楽"]);
    expect(parseTags("  ")).toEqual([]);
  });

  it("an article's ID, bare or inside the address of its page", () => {
    const id = "01a200cd-8ef4-753d-aefe-1ea2d6c2fbdd";
    expect(articleIdFrom(id)).toBe(id);
    expect(articleIdFrom(`https://tokuly.example/news/${id}?from=share`)).toBe(id);
    expect(articleIdFrom(`/news/articles/${id.toUpperCase()}`)).toBe(id);
    expect(articleIdFrom("/news/sources")).toBeUndefined();
  });
});
