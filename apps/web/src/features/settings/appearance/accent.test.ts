import { colors } from "@tokuly/core/tokens";
import { describe, expect, it } from "vitest";
import { ACCENTS, accentTokens, DEFAULT_ACCENT, isAccentColor } from "./accent";

/** WCAG contrast ratio between two #rrggbb colours. */
function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const [r, g, b] = [1, 3, 5].map((start) => {
      const value = Number.parseInt(hex.slice(start, start + 2), 16) / 255;
      return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    }) as [number, number, number];
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

describe("accent colours", () => {
  it("accepts only #rrggbb", () => {
    expect(isAccentColor("#7C5cff")).toBe(true);
    expect(isAccentColor("#fff")).toBe(false);
    expect(isAccentColor("red")).toBe(false);
    expect(isAccentColor("#7c5cff; background: url(x)")).toBe(false);
  });

  it("deepens the light theme's brand only as far as small text needs", () => {
    expect(accentTokens("#4a4266").light.brand).toBe("#4a4266");
    expect(accentTokens("#7c5cff").light.brand).toBe("#7859f7");
    expect(accentTokens("#e07a00").light.brand).toBe("#b36200");
  });

  it("gives the shipped colour the shipped brand", () => {
    expect(accentTokens(DEFAULT_ACCENT).light.brand).toBe(colors.light.brand);
  });

  it("deepens a pale accent until white text reads on it", () => {
    const { light } = accentTokens("#ffe14d");
    expect(light.onBrand).toBe("#ffffff");
    expect(contrast(light.brand, light.onBrand)).toBeGreaterThanOrEqual(4.5);
  });

  it("offers the shipped colour first", () => {
    expect(ACCENTS[0].color).toBe(DEFAULT_ACCENT);
  });

  it.each(ACCENTS.map((accent) => [accent.id, accent.color] as const))(
    "%s stays readable in both themes",
    (_id, color) => {
      const { light, dark } = accentTokens(color);
      // Labels on the brand colour, and brand-coloured text on the surfaces: small text, 4.5:1.
      expect(contrast(light.brand, light.onBrand)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(dark.brand, dark.onBrand)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(light.brand, colors.light.surface)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(dark.brand, colors.dark.surface)).toBeGreaterThanOrEqual(4.5);
    },
  );
});
