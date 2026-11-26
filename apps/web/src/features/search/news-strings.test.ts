import { describe, expect, it } from "vitest";
import enNews from "#/i18n/en/news";
import jaNews from "#/i18n/ja/news";

/** Every leaf of a strings file as "a.b.c". */
function paths(value: unknown, prefix = ""): string[] {
  if (typeof value === "string") return [prefix];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    paths(child, prefix ? `${prefix}.${key}` : key),
  );
}

/** The `{{placeholders}}` a string expects. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1] ?? "").sort();
}

function at(strings: unknown, path: string): string {
  return path
    .split(".")
    .reduce<unknown>((node, key) => (node as Record<string, unknown>)[key], strings) as string;
}

describe("news strings", () => {
  it("have the same keys in Japanese and English", () => {
    expect(paths(enNews).sort()).toEqual(paths(jaNews).sort());
  });

  it("use the same placeholders in both languages", () => {
    for (const path of paths(jaNews))
      expect(placeholders(at(enNews, path)), path).toEqual(placeholders(at(jaNews, path)));
  });
});
