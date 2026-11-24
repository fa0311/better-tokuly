import { describe, expect, it } from "vitest";
import enCircles from "#/i18n/en/circles";
import enFollows from "#/i18n/en/follows";
import enFriends from "#/i18n/en/friends";
import jaCircles from "#/i18n/ja/circles";
import jaFollows from "#/i18n/ja/follows";
import jaFriends from "#/i18n/ja/friends";
import { CIRCLE_COLORS } from "./follow-data";

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

describe.each([
  ["friends", jaFriends, enFriends],
  ["follows", jaFollows, enFollows],
  ["circles", jaCircles, enCircles],
])("%s strings", (_name, ja, en) => {
  it("have the same keys in Japanese and English", () => {
    expect(paths(en).sort()).toEqual(paths(ja).sort());
  });

  it("use the same placeholders in both languages", () => {
    for (const path of paths(ja))
      expect(placeholders(at(en, path)), path).toEqual(placeholders(at(ja, path)));
  });
});

describe("circle colours", () => {
  it("each have a name in both languages", () => {
    for (const { key } of CIRCLE_COLORS) {
      expect(jaCircles.colors).toHaveProperty(key);
      expect(enCircles.colors).toHaveProperty(key);
    }
  });
});
