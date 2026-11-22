import { describe, expect, it } from "vitest";
import enEngagement from "#/i18n/en/engagement";
import enNotes from "#/i18n/en/notes";
import enStories from "#/i18n/en/stories";
import jaEngagement from "#/i18n/ja/engagement";
import jaNotes from "#/i18n/ja/notes";
import jaStories from "#/i18n/ja/stories";

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
  ["notes", jaNotes, enNotes],
  ["stories", jaStories, enStories],
  ["engagement", jaEngagement, enEngagement],
])("%s strings", (_name, ja, en) => {
  it("has the same keys in Japanese and English", () => {
    expect(paths(en).sort()).toEqual(paths(ja).sort());
  });

  it("uses the same placeholders in both languages", () => {
    for (const path of paths(ja)) expect(placeholders(at(en, path)), path).toEqual(placeholders(at(ja, path)));
  });
});
