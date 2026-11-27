import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { tokensCss } from "./tokens-css";

describe("tokens.css", () => {
  it("is what @tokuly/core/tokens says (out of date: `pnpm --filter @tokuly/web tokens`)", () => {
    const written = readFileSync(new URL("../tokens.css", import.meta.url), "utf8");
    expect(written).toBe(tokensCss());
  });
});
