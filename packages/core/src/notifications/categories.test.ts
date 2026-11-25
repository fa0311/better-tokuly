import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { en, ja } from "@tokuly/i18n";
import { describe, expect, it } from "vitest";
import { NOTIFY_CATEGORIES } from "./categories";

/** The category IDs the server defines, read from its source. */
function serverCategories(): string[] {
  const source = readFileSync(
    fileURLToPath(new URL("../../../../backend/crates/notify/src/categories.rs", import.meta.url)),
    "utf8",
  );
  const list = source.slice(source.indexOf("pub const CATEGORIES"));
  const system = /const SYSTEM: &str = "([a-z]+)";/.exec(source)?.[1];
  return [...list.matchAll(/^\s+id: (?:"([a-z]+)"|SYSTEM),$/gm)].map(
    (match) => match[1] ?? system!,
  );
}

describe("notification categories", () => {
  it("are the server's, in its order", () => {
    expect([...NOTIFY_CATEGORIES]).toEqual(serverCategories());
  });

  it("each have a channel name on the phone, in every language", () => {
    for (const strings of [ja, en])
      expect(Object.keys(strings.mobile.push.channels).sort()).toEqual(
        [...NOTIFY_CATEGORIES].sort(),
      );
  });
});
