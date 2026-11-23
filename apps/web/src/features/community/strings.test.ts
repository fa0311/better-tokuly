import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import en from "#/i18n/en/community";
import ja from "#/i18n/ja/community";

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

/** Every `.ts` / `.tsx` source of this feature (tests aside). */
function sources(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sources(path);
    return /\.tsx?$/.test(entry.name) && !entry.name.includes(".test.") ? [path] : [];
  });
}

describe("community strings", () => {
  it("has the same keys in Japanese and English", () => {
    expect(paths(en).sort()).toEqual(paths(ja).sort());
  });

  it("uses the same placeholders in both languages", () => {
    for (const path of paths(ja))
      expect(placeholders(at(en, path)), path).toEqual(placeholders(at(ja, path)));
  });

  it("has every key the screens ask for", () => {
    const known = new Set(paths(ja));
    const here = dirname(fileURLToPath(import.meta.url));
    const missing = sources(here).flatMap((file) =>
      [...readFileSync(file, "utf8").matchAll(/\bt\(\s*"([A-Za-z0-9_.]+)"/g)]
        .map((match) => match[1] ?? "")
        .filter((key) => !known.has(key))
        .map((key) => `${file.slice(here.length + 1)}: ${key}`),
    );
    expect(missing).toEqual([]);
  });
});
