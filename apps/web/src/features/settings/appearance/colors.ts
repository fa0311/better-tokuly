import { create, identity } from "@tokuly/api";
import {
  cleanColors,
  hasCustomColors,
  NO_CUSTOM_COLORS,
  type ThemeColors,
  type ThemeName,
} from "@tokuly/core/theme";
import { type Colors, colors } from "@tokuly/core/tokens";
import { COLORS_KEY, COLORS_STYLE_ID } from "#/lib/appearance";
import { accentTokens, DEFAULT_ACCENT, isAccentColor } from "./accent";

/**
 * The account's colours, applied on top of the theme: the brand tokens of
 * its accent colour, then what it recoloured in the theme editor. Both
 * redefine the tokens of `tokens.css` in one small stylesheet of our own, so
 * every `bg-brand` / `text-ink` in the app follows. The stylesheet is kept
 * for the next visit, whose boot script applies it before the first paint.
 */
export interface AccountColors {
  accent: string;
  custom: ThemeColors;
}

export const DEFAULT_COLORS: AccountColors = { accent: DEFAULT_ACCENT, custom: NO_CUSTOM_COLORS };

function customColors(value: unknown): ThemeColors {
  const themes = (value && typeof value === "object" ? value : {}) as Partial<ThemeColors>;
  return { light: cleanColors(themes.light), dark: cleanColors(themes.dark) };
}

/** The colours of the account's appearance settings. */
export function accountColors(appearance: identity.AppearanceSettings): AccountColors {
  const accent = appearance.accentColor.toLowerCase();
  return {
    accent: isAccentColor(accent) ? accent : DEFAULT_ACCENT,
    custom: customColors(appearance.customColors),
  };
}

/** The theme editor's colours as the settings store them. */
export function customColorsMessage(custom: ThemeColors): identity.ThemeColors {
  return create(identity.ThemeColorsSchema, custom as Record<ThemeName, Record<string, string>>);
}

/** A theme as the app shows it before the theme editor: the shipped colours with the accent's brand. */
export function baseColors(accent: string, theme: ThemeName): Colors {
  if (accent === DEFAULT_ACCENT) return colors[theme];
  const brand = accentTokens(accent)[theme];
  return {
    ...colors[theme],
    brand: brand.brand,
    "brand-strong": brand.brandStrong,
    "brand-soft": brand.brandSoft,
    "on-brand": brand.onBrand,
  };
}

/** Every token of a theme as the account sees it. */
export function themeColors({ accent, custom }: AccountColors, theme: ThemeName): Colors {
  return { ...baseColors(accent, theme), ...custom[theme] };
}

/** `--bg:#…;--ink:#…;…` for every token of a theme. */
export function cssVariables(theme: Colors): string {
  return Object.entries(theme)
    .map(([name, color]) => `--${name}:${color};`)
    .join("");
}

/** The stylesheet of the account's colours: every token in both rules, so a colour changed for the light theme only does not reach the dark one. */
export function colorsCss(look: AccountColors): string {
  return (
    `:root:root{${cssVariables(themeColors(look, "light"))}}` +
    `:root:root[data-theme="dark"]{${cssVariables(themeColors(look, "dark"))}}`
  );
}

/** Applies the account's colours to the whole app and remembers them for the next visit. */
export function applyColors(look: AccountColors): void {
  const existing = document.getElementById(COLORS_STYLE_ID);
  if (look.accent === DEFAULT_ACCENT && !hasCustomColors(look.custom)) {
    existing?.remove();
    window.localStorage.removeItem(COLORS_KEY);
    return;
  }
  const css = colorsCss(look);
  const style =
    existing ??
    document.head.appendChild(
      Object.assign(document.createElement("style"), { id: COLORS_STYLE_ID }),
    );
  style.textContent = css;
  window.localStorage.setItem(COLORS_KEY, css);
}
