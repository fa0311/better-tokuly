import { describe, expect, it } from "vitest";
import enNews from "#/i18n/en/news";
import enSearch from "#/i18n/en/search";
import jaNews from "#/i18n/ja/news";
import jaSearch from "#/i18n/ja/search";
import { KINDS, kindSlug } from "./kinds";

/** Every leaf of a strings file as "a.b.c". */
function paths(value: unknown, prefix = ""): string[] {
  if (typeof value === "string") return [prefix];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) => paths(child, prefix ? `${prefix}.${key}` : key));
}

/** The `{{placeholders}}` a string expects. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1] ?? "").sort();
}

function at(strings: unknown, path: string): string {
  return path.split(".").reduce<unknown>((node, key) => (node as Record<string, unknown>)[key], strings) as string;
}

describe.each([
  ["search", jaSearch, enSearch],
  ["news", jaNews, enNews],
])("%s strings", (_name, ja, en) => {
  it("have the same keys in Japanese and English", () => {
    expect(paths(en).sort()).toEqual(paths(ja).sort());
  });

  it("use the same placeholders in both languages", () => {
    for (const path of paths(ja)) expect(placeholders(at(en, path)), path).toEqual(placeholders(at(ja, path)));
  });
});

describe("search strings", () => {
  it("name every kind of result", () => {
    for (const kind of KINDS) expect(jaSearch.kinds).toHaveProperty(kindSlug(kind));
  });
});
