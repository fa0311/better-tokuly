import { describe, expect, it } from "vitest";
import { colors, overlay } from "./index";

describe("design tokens", () => {
  it("name the same colours in both themes, each as #rrggbb", () => {
    expect(Object.keys(colors.dark)).toEqual(Object.keys(colors.light));
    for (const theme of [
      colors.light,
      colors.dark,
      { a: overlay.light.color, b: overlay.dark.color },
    ])
      for (const value of Object.values(theme)) expect(value).toMatch(/^#[0-9a-f]{6}$/);
  });
});
