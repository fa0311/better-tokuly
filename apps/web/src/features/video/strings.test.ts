import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import enVideo from "@tokuly/i18n/en/video";
import jaVideo from "@tokuly/i18n/ja/video";
import { describe, expect, it } from "vitest";
import enComments from "#/i18n/en/comments";
import jaComments from "#/i18n/ja/comments";

type Strings = { [key: string]: string | Strings };

/** Every leaf of a strings file as a dotted key. */
function keysOf(strings: Strings, prefix = ""): string[] {
  return Object.entries(strings).flatMap(([key, value]) =>
    typeof value === "string" ? [prefix + key] : keysOf(value, `${prefix}${key}.`),
  );
}

function at(strings: Strings, key: string): string {
  return key
    .split(".")
    .reduce<string | Strings>(
      (node, part) => (node as Strings)[part] as string | Strings,
      strings,
    ) as string;
}

/** The `{{placeholders}}` a string expects. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1] ?? "").sort();
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.tsx?$/.test(name) && !name.includes(".test.") ? [path] : [];
  });
}

/**
 * Keys a feature's code asks for: the first argument of a literal `t("...")`
 * and every other string literal shaped like a dotted key of the namespace
 * (the branches of `t(open ? "a.b" : "a.c")`, keys kept in tables). Keys
 * built with template strings are not seen.
 */
function usedKeys(directory: string, strings: Strings): string[] {
  const groups = new Set(Object.keys(strings));
  const used = new Set<string>();
  for (const file of sourceFiles(directory)) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\bt\(\s*"([A-Za-z0-9_.]+)"/g))
      used.add(match[1] as string);
    for (const match of source.matchAll(/"([A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+)"/g)) {
      const literal = match[1] as string;
      if (groups.has(literal.split(".")[0] as string)) used.add(literal);
    }
  }
  return [...used];
}

describe.each([
  ["video", jaVideo as Strings, enVideo as Strings, "src/features/video"],
  ["comments", jaComments as Strings, enComments as Strings, "src/features/comments"],
])("%s strings", (_namespace, ja, en, directory) => {
  it("have the same keys in Japanese and English", () => {
    expect(keysOf(en).sort()).toEqual(keysOf(ja).sort());
  });

  it("use the same placeholders in both languages", () => {
    for (const key of keysOf(ja))
      expect(placeholders(at(en, key)), key).toEqual(placeholders(at(ja, key)));
  });

  it("define every key the feature's code asks for", () => {
    const defined = new Set(keysOf(ja));
    expect(usedKeys(directory, ja).filter((key) => !defined.has(key))).toEqual([]);
  });
});
