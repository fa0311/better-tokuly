import { describe, expect, it } from "vitest";
import en from "#/i18n/en/settings";
import ja from "#/i18n/ja/settings";

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

describe("settings strings", () => {
  it("has the same keys in Japanese and English", () => {
    expect(paths(en).sort()).toEqual(paths(ja).sort());
  });

  it("uses the same placeholders in both languages", () => {
    for (const path of paths(ja)) expect(placeholders(at(en, path)), path).toEqual(placeholders(at(ja, path)));
  });
});
