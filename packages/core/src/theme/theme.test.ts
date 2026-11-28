import { describe, expect, it } from "vitest";
import { colors } from "../tokens";
import {
  cleanColors,
  contrastProblems,
  contrastRatio,
  NO_CUSTOM_COLORS,
  parseThemeFile,
  TOKEN_GROUPS,
  themeFile,
  withoutBrand,
} from ".";

describe("custom theme colours", () => {
  it("offers every design token once", () => {
    const offered = TOKEN_GROUPS.flatMap(({ tokens }) => tokens);
    expect(new Set(offered).size).toBe(offered.length);
    expect([...offered].sort()).toEqual(Object.keys(colors.light).sort());
  });

  it("keeps known tokens with a valid colour, and only what differs from the base", () => {
    expect(
      cleanColors(
        { ink: "#ABCDEF", bg: colors.light.bg, nope: "#000000", line: "red", sky: 3 },
        colors.light,
      ),
    ).toEqual({ ink: "#abcdef" });
    expect(Object.keys(cleanColors({ sky: "#000000", bg: "#000000", ink: "#000000" }))).toEqual([
      "bg",
      "ink",
      "sky",
    ]);
    expect(cleanColors(null)).toEqual({});
    expect(cleanColors({ __proto__: "#000000", toString: "#000000" })).toEqual({});
  });

  it("hands the brand tokens back to the accent colour", () => {
    expect(
      withoutBrand({
        light: { brand: "#000000", ink: "#111111" },
        dark: { "on-brand": "#ffffff" },
      }),
    ).toEqual({ light: { ink: "#111111" }, dark: {} });
  });
});

describe("contrast", () => {
  it("measures WCAG ratios", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21);
    expect(contrastRatio("#ffffff", "#ffffff")).toBe(1);
    expect(contrastRatio("#767676", "#ffffff")).toBeCloseTo(4.54, 2);
  });

  it("finds nothing to warn about in the shipped themes", () => {
    expect(contrastProblems(colors.light)).toEqual([]);
    expect(contrastProblems(colors.dark)).toEqual([]);
  });

  it("warns about text that is too faint for its background", () => {
    const problems = contrastProblems({ ...colors.light, "ink-soft": "#bbbbbb" });
    expect(problems.map(({ text, on }) => `${text}/${on}`)).toEqual([
      "ink-soft/bg",
      "ink-soft/surface",
      "ink-soft/surface-2",
      "ink-soft/surface-3",
    ]);
    expect(problems[0]?.min).toBe(4.5);
  });

  it("checks the faintest text where it is text, not on surface-3 where it is only icons", () => {
    const problems = contrastProblems({ ...colors.light, "ink-faint": "#9993ad" });
    expect(problems.map(({ text, on }) => `${text}/${on}`)).toEqual([
      "ink-faint/bg",
      "ink-faint/surface",
      "ink-faint/surface-2",
    ]);
  });
});

describe("theme files", () => {
  it("round-trip", () => {
    const custom = { light: { ink: "#101010" }, dark: { bg: "#000000", lime: "#aaff00" } };
    expect(parseThemeFile(themeFile(custom))).toEqual({ colors: custom, skipped: 0 });
    expect(parseThemeFile(themeFile(NO_CUSTOM_COLORS))).toEqual({
      colors: NO_CUSTOM_COLORS,
      skipped: 0,
    });
  });

  it("leaves out what this app does not know, and says so", () => {
    const text = JSON.stringify({
      format: "tokuly-theme",
      version: 1,
      light: { ink: "#101010", sparkle: "#ffffff", bg: "blue" },
    });
    expect(parseThemeFile(text)).toEqual({
      colors: { light: { ink: "#101010" }, dark: {} },
      skipped: 2,
    });
  });

  it("refuses what is not a theme file", () => {
    expect(parseThemeFile("{")).toEqual({ error: "notJson" });
    expect(parseThemeFile('{"light":{}}')).toEqual({ error: "notTheme" });
    expect(parseThemeFile("null")).toEqual({ error: "notTheme" });
    expect(parseThemeFile('{"format":"tokuly-theme","version":2}')).toEqual({
      error: "newerVersion",
    });
  });
});
