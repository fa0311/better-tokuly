import { describe, expect, it } from "vitest";
import { BOOT_SCRIPT, COLORS_CSS, COLORS_KEY } from "#/lib/appearance";
import { colorsCss } from "./colors";

/** Runs the boot script against a stored value; what it put into <head>, if anything. */
function boot(stored: string | null): { id: string; textContent: string }[] {
  const head: { id: string; textContent: string }[] = [];
  const document = {
    documentElement: { dataset: {} },
    createElement: () => ({ id: "", textContent: "" }),
    head: { appendChild: (node: { id: string; textContent: string }) => head.push(node) },
  };
  const localStorage = { getItem: (key: string) => (key === COLORS_KEY ? stored : null) };
  const matchMedia = () => ({ matches: false });
  new Function("document", "localStorage", "matchMedia", BOOT_SCRIPT)(
    document,
    localStorage,
    matchMedia,
  );
  return head;
}

describe("the account's colours before the first paint", () => {
  const css = colorsCss({
    accent: "#12a36b",
    custom: { light: { bg: "#fff4e0" }, dark: { ink: "#ffe0f0" } },
  });

  it("are a stylesheet of token variables the boot script accepts", () => {
    expect(css).toMatch(COLORS_CSS);
    expect(css).toContain("--bg:#fff4e0;");
    expect(boot(css)).toEqual([{ id: "tokuly-colors", textContent: css }]);
  });

  it("are not applied when storage holds anything else", () => {
    expect(boot(null)).toEqual([]);
    for (const tampered of [
      ":root:root{--bg:url(https://example.com/x);}",
      ":root:root{--bg:#000000;}body{display:none}",
      `${css}</style><script>alert(1)</script>`,
      ":root{--bg:#000000;}",
    ])
      expect(boot(tampered)).toEqual([]);
  });
});
