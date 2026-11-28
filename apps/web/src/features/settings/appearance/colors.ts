import { create, identity } from "@tokuly/api";
import {
  cleanColors,
  hasCustomColors,
  NO_CUSTOM_COLORS,
  type ThemeColors,
  type ThemeName,
} from "@tokuly/core/theme";
import { type Colors, colors } from "@tokuly/core/tokens";
import { accentTokens, DEFAULT_ACCENT, isAccentColor } from "./accent";

/**
 * The account's colours, applied on top of the theme: the brand tokens of
 * its accent colour, then what it recoloured in the theme editor. Both
 * redefine the tokens of `tokens.css` in one small stylesheet of our own, so
 * every `bg-brand` / `text-ink` in the app follows. The last applied colours
 * are kept for the next visit, so it starts in them.
 */
export interface AccountColors {
  accent: string;
  custom: ThemeColors;
}

export const DEFAULT_COLORS: AccountColors = { accent: DEFAULT_ACCENT, custom: NO_CUSTOM_COLORS };

const ACCENT_KEY = "tokuly.accent";
const CUSTOM_KEY = "tokuly.customColors";
const STYLE_ID = "tokuly-colors";

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

/** Applies the account's colours to the whole app and remembers them for the next visit. */
export function applyColors(look: AccountColors): void {
  const existing = document.getElementById(STYLE_ID);
  if (look.accent === DEFAULT_ACCENT && !hasCustomColors(look.custom)) {
    existing?.remove();
    window.localStorage.removeItem(ACCENT_KEY);
    window.localStorage.removeItem(CUSTOM_KEY);
    return;
  }
  const style =
    existing ??
    document.head.appendChild(Object.assign(document.createElement("style"), { id: STYLE_ID }));
  // Every token in both rules: a colour changed for the light theme only must not reach the dark one.
  style.textContent =
    `:root{${cssVariables(themeColors(look, "light"))}}` +
    `:root[data-theme="dark"]{${cssVariables(themeColors(look, "dark"))}}`;
  window.localStorage.setItem(ACCENT_KEY, look.accent);
  window.localStorage.setItem(CUSTOM_KEY, JSON.stringify(look.custom));
}

/** The colours applied on the last visit (the defaults when none were chosen). */
export function storedColors(): AccountColors {
  const accent = window.localStorage.getItem(ACCENT_KEY);
  let custom: unknown = null;
  try {
    custom = JSON.parse(window.localStorage.getItem(CUSTOM_KEY) ?? "null");
  } catch {}
  return {
    accent: accent && isAccentColor(accent) ? accent : DEFAULT_ACCENT,
    custom: customColors(custom),
  };
}
