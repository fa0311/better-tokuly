import { type ColorName, type Colors, colors } from "../tokens";

/**
 * Custom themes: the colours an account recoloured with the theme editor,
 * over the design tokens (`@tokuly/core/tokens`). Only the tokens that differ
 * are kept, per theme; the account's settings store them
 * (`AppearanceSettings.custom_colors`) and a theme file carries them between
 * accounts.
 */
export type ThemeName = "light" | "dark";
export type ThemeColors = Record<ThemeName, Partial<Colors>>;

export const NO_CUSTOM_COLORS: ThemeColors = { light: {}, dark: {} };

/** The tokens the editor offers, in groups (names are `appearance.editor.groups.<group>`). */
export const TOKEN_GROUPS = [
  { group: "surfaces", tokens: ["bg", "surface", "surface-2", "surface-3", "line", "shadow"] },
  { group: "text", tokens: ["ink", "ink-soft", "ink-faint"] },
  { group: "brand", tokens: ["brand", "brand-strong", "brand-soft", "on-brand"] },
  { group: "fills", tokens: ["lime", "pink", "sky", "sun", "coral", "mint"] },
  { group: "states", tokens: ["danger", "success", "live"] },
] as const satisfies readonly { group: string; tokens: readonly ColorName[] }[];

/** The tokens an accent colour sets; choosing an accent hands them back to it. */
export const BRAND_TOKENS: readonly ColorName[] = [
  "brand",
  "brand-strong",
  "brand-soft",
  "on-brand",
];

export function isHexColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value);
}

/**
 * The known tokens with a valid colour (lower case), from anything (stored
 * settings, a file). With `base`, a colour equal to the base one is dropped:
 * it is not a change.
 */
export function cleanColors(value: unknown, base?: Colors): Partial<Colors> {
  if (!value || typeof value !== "object") return {};
  const clean: Partial<Colors> = {};
  // In the tokens' own order, whatever order they came in: equal colours make equal JSON.
  for (const name of Object.keys(colors.light) as ColorName[]) {
    const color: unknown = Object.hasOwn(value, name)
      ? (value as Record<string, unknown>)[name]
      : null;
    if (typeof color !== "string" || !isHexColor(color)) continue;
    const lower = color.toLowerCase();
    if (base?.[name] !== lower) clean[name] = lower;
  }
  return clean;
}

/** `custom` without the tokens an accent colour sets. */
export function withoutBrand(custom: ThemeColors): ThemeColors {
  const drop = (theme: Partial<Colors>) =>
    Object.fromEntries(
      Object.entries(theme).filter(([name]) => !BRAND_TOKENS.includes(name as ColorName)),
    ) as Partial<Colors>;
  return { light: drop(custom.light), dark: drop(custom.dark) };
}

export function hasCustomColors(custom: ThemeColors): boolean {
  return Object.keys(custom.light).length > 0 || Object.keys(custom.dark).length > 0;
}

// ── Contrast ────────────────────────────────────────────────────────

/** WCAG relative luminance of a #rrggbb colour, 0 (black) … 1 (white). */
export function luminance(hex: string): number {
  const [r = 0, g = 0, b = 0] = [1, 3, 5].map((start) => {
    const value = Number.parseInt(hex.slice(start, start + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio of two #rrggbb colours, 1 … 21. */
export function contrastRatio(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

/** WCAG AA (SC 1.4.3) for text below 18.66px bold / 24px regular. */
export const AA_TEXT = 4.5;

/**
 * Text on the backgrounds it is set on, each at the WCAG AA ratio it needs.
 * Every pair carries small text somewhere, so every pair needs 4.5:1:
 * `ink` and `ink-soft` everywhere; `ink-faint` (times, counts, hints,
 * placeholders) on the page, cards and fields, while on `surface-3` it is
 * only icons; `on-brand` on `brand` in buttons, badges and the selected
 * navigation label; `brand` on `surface` in links, chips and 10–14px
 * labels. AA's 3:1 is for text at 18.66px bold / 24px and for icons only,
 * and no pair here is used only that way.
 */
export const CONTRAST_CHECKS = [
  ...(["ink", "ink-soft"] as const).flatMap((text) =>
    (["bg", "surface", "surface-2", "surface-3"] as const).map((on) => ({
      text,
      on,
      min: AA_TEXT,
    })),
  ),
  ...(["bg", "surface", "surface-2"] as const).map((on) => ({
    text: "ink-faint" as const,
    on,
    min: AA_TEXT,
  })),
  { text: "on-brand", on: "brand", min: AA_TEXT },
  { text: "brand", on: "surface", min: AA_TEXT },
] as const satisfies readonly { text: ColorName; on: ColorName; min: number }[];

export interface ContrastProblem {
  text: ColorName;
  on: ColorName;
  ratio: number;
  min: number;
}

/** The pairs of `CONTRAST_CHECKS` that `theme` (every token) leaves below AA. */
export function contrastProblems(theme: Colors): ContrastProblem[] {
  return CONTRAST_CHECKS.flatMap(({ text, on, min }) => {
    const ratio = contrastRatio(theme[text], theme[on]);
    return ratio < min ? [{ text, on, ratio, min }] : [];
  });
}

// ── Theme files ─────────────────────────────────────────────────────

const FORMAT = "tokuly-theme";
const VERSION = 1;

/** A theme file: the custom colours as JSON, readable and editable by hand. */
export function themeFile(custom: ThemeColors): string {
  return `${JSON.stringify({ format: FORMAT, version: VERSION, ...custom }, null, 2)}\n`;
}

export type ThemeFileError = "notJson" | "notTheme" | "newerVersion";

/**
 * The colours of a theme file. Unknown tokens and invalid colours are left
 * out (a file from a newer app may name more tokens); `skipped` counts them.
 */
export function parseThemeFile(
  text: string,
): { colors: ThemeColors; skipped: number } | { error: ThemeFileError } {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { error: "notJson" };
  }
  if (!value || typeof value !== "object" || (value as { format?: unknown }).format !== FORMAT)
    return { error: "notTheme" };
  const file = value as { version?: unknown; light?: unknown; dark?: unknown };
  if (typeof file.version === "number" && file.version > VERSION) return { error: "newerVersion" };
  const parsed = { light: cleanColors(file.light), dark: cleanColors(file.dark) };
  const given = [file.light, file.dark].reduce<number>(
    (count, theme) => count + (theme && typeof theme === "object" ? Object.keys(theme).length : 0),
    0,
  );
  const kept = Object.keys(parsed.light).length + Object.keys(parsed.dark).length;
  return { colors: parsed, skipped: given - kept };
}
