import { describe, expect, it } from "vitest";
import { en, ja, NAMESPACES, normalizeLanguage } from "./index";

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

it("lists every namespace for loading one at a time", () => {
  expect([...NAMESPACES].sort()).toEqual(Object.keys(ja).sort());
  expect(Object.keys(en).sort()).toEqual(Object.keys(ja).sort());
});

describe.each(NAMESPACES)("%s strings", (namespace) => {
  it("has the same keys in Japanese and English", () => {
    expect(paths(en[namespace]).sort()).toEqual(paths(ja[namespace]).sort());
  });

  it("asks for the same placeholders in both languages", () => {
    for (const path of paths(ja[namespace]))
      expect(placeholders(at(en[namespace], path)), path).toEqual(
        placeholders(at(ja[namespace], path)),
      );
  });
});

describe("normalizeLanguage", () => {
  it("prefers Japanese and falls back to English", () => {
    expect(normalizeLanguage(undefined)).toBe("ja");
    expect(normalizeLanguage("ja-JP")).toBe("ja");
    expect(normalizeLanguage("en-US")).toBe("en");
    expect(normalizeLanguage("fr")).toBe("en");
  });
});
