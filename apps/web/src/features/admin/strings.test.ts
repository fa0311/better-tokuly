import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import enAdmin from "#/i18n/en/admin";
import enAnnouncements from "#/i18n/en/announcements";
import jaAdmin from "#/i18n/ja/admin";
import jaAnnouncements from "#/i18n/ja/announcements";

type Strings = { [key: string]: string | Strings };

/** Every leaf of a strings file as a dotted key. */
function keysOf(strings: Strings, prefix = ""): string[] {
  return Object.entries(strings).flatMap(([key, value]) => (typeof value === "string" ? [prefix + key] : keysOf(value, `${prefix}${key}.`)));
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.tsx?$/.test(name) && !name.includes(".test.") ? [path] : [];
  });
}

/**
 * Keys a feature's code asks for: the first argument of a literal `t("...")`,
 * plus every other string literal shaped like a dotted key of the namespace
 * (the branches of `t(open ? "a.b" : "a.c")`). Keys built with template
 * strings are not seen. `namespace:` prefixes of this namespace are dropped.
 */
function usedKeys(directory: string, namespace: string, strings: Strings): string[] {
  const groups = new Set(Object.keys(strings));
  const used = new Set<string>();
  const add = (literal: string) => {
    if (!literal.includes(":")) used.add(literal);
    else if (literal.startsWith(`${namespace}:`)) used.add(literal.slice(namespace.length + 1));
  };
  for (const file of sourceFiles(directory)) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\bt\(\s*"([A-Za-z0-9_.:]+)"/g)) add(match[1] as string);
    for (const match of source.matchAll(/"((?:[a-z]+:)?[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+)"/g)) {
      const literal = match[1] as string;
      if (groups.has((literal.split(":").pop() as string).split(".")[0] as string)) add(literal);
    }
  }
  return [...used];
}

describe("admin and announcements strings", () => {
  it("have the same keys in Japanese and English", () => {
    expect(keysOf(enAdmin).sort()).toEqual(keysOf(jaAdmin).sort());
    expect(keysOf(enAnnouncements).sort()).toEqual(keysOf(jaAnnouncements).sort());
  });

  it("define every key the admin console asks for", () => {
    const defined = new Set(keysOf(jaAdmin));
    // The console also reads a few shared strings; those are not this namespace's to define.
    const shared = (key: string) => key.startsWith("actions.");
    expect(usedKeys("src/features/admin", "admin", jaAdmin).filter((key) => !defined.has(key) && !shared(key))).toEqual([]);
  });

  it("define every key the update log asks for", () => {
    const defined = new Set(keysOf(jaAnnouncements));
    expect(usedKeys("src/features/announcements", "announcements", jaAnnouncements).filter((key) => !defined.has(key))).toEqual([]);
  });
});
