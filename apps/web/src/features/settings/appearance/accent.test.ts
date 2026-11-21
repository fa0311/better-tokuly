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

  it("keeps the light theme's brand as the chosen colour", () => {
    expect(accentTokens("#ec3f8c").light.brand).toBe("#ec3f8c");
  });

  it("picks dark text for a pale accent and white for a deep one", () => {
    expect(accentTokens("#ffe14d").light.onBrand).toBe("#17122b");
    expect(accentTokens("#4a4266").light.onBrand).toBe("#ffffff");
  });

  it("offers the shipped colour first", () => {
    expect(ACCENTS[0].color).toBe(DEFAULT_ACCENT);
  });

  it.each(ACCENTS.map((accent) => [accent.id, accent.color] as const))("%s stays readable in both themes", (_id, color) => {
    const { light, dark } = accentTokens(color);
    // Button labels on the brand colour.
    expect(contrast(light.brand, light.onBrand)).toBeGreaterThanOrEqual(3);
    expect(contrast(dark.brand, dark.onBrand)).toBeGreaterThanOrEqual(4.5);
    // Brand-coloured text and icons on the surfaces of each theme.
    expect(contrast(light.brand, "#ffffff")).toBeGreaterThanOrEqual(3);
    expect(contrast(dark.brand, "#17132a")).toBeGreaterThanOrEqual(4.5);
  });
});
