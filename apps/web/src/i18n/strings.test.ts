import { describe, expect, it } from "vitest";

const ja = import.meta.glob<{ default: object }>("./ja/*.ts", { eager: true });
const en = import.meta.glob<{ default: object }>("./en/*.ts", { eager: true });

/** Every leaf key, plural suffixes folded (the languages pluralize differently). */
function keys(strings: object, prefix = ""): string[] {
  return Object.entries(strings).flatMap(([key, value]) =>
    value && typeof value === "object"
      ? keys(value, `${prefix}${key}.`)
      : [`${prefix}${key}`.replace(/_(zero|one|two|few|many|other)$/, "")],
  );
}

// The browser loads one language and no fallback (`#/lib/i18n`): a key only Japanese has would show as the key itself.
describe.each(Object.keys(ja))("%s", (file) => {
  it("has every Japanese key in English", () => {
    const japanese = keys(ja[file]?.default ?? {});
    const english = new Set(keys(en[file.replace("./ja/", "./en/")]?.default ?? {}));
    const missing = japanese.filter((key) => !english.has(key));
    expect(missing).toEqual([]);
  });
});
