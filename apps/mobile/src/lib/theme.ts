import { vars } from "nativewind";
import { createContext, useContext } from "react";
import { useColorScheme } from "react-native";

/**
 * The web app's design tokens ("sticker pop", `apps/web/src/styles.css`),
 * copied: lavender paper, ink outlines, hard shadows, a purple brand with
 * lime, pink, sky and sun next to it. Components use the Tailwind names
 * (`bg-surface`, `text-ink-soft`); native props that take a colour (icons,
 * the status bar, the map) read `useTheme().colors`.
 */
const LIGHT = {
  bg: "#f4f1ff",
  surface: "#ffffff",
  "surface-2": "#f0ecfb",
  "surface-3": "#e4def5",
  ink: "#17122b",
  "ink-soft": "#5d5778",
  "ink-faint": "#9993ad",
  line: "#ddd6ee",
  shadow: "#17122b",
  brand: "#7c5cff",
  "brand-strong": "#5f3df0",
  "brand-soft": "#e9e3ff",
  "on-brand": "#ffffff",
  lime: "#c6f24e",
  pink: "#ff5c9d",
  sky: "#4fc3ff",
  sun: "#ffd23f",
  coral: "#ff7a59",
  mint: "#3ddc97",
  danger: "#f0425f",
  success: "#1fb57a",
  live: "#ff2d55",
};

export type ColorName = keyof typeof LIGHT;
export type Colors = Record<ColorName, string>;

const DARK: Colors = {
  ...LIGHT,
  bg: "#0d0b14",
  surface: "#17132a",
  "surface-2": "#211c3a",
  "surface-3": "#2d274c",
  ink: "#f6f3ff",
  "ink-soft": "#b4adcf",
  "ink-faint": "#7a7397",
  line: "#322b52",
  shadow: "#000000",
  brand: "#9a82ff",
  "brand-strong": "#b7a5ff",
  "brand-soft": "#2a2152",
  "on-brand": "#120c2b",
};

/** `#rrggbb` → `r g b`, the form `rgb(var(--x) / <alpha-value>)` in tailwind.config.js takes. */
function channels(hex: string): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `${(value >> 16) & 255} ${(value >> 8) & 255} ${value & 255}`;
}

function cssVariables(colors: Colors) {
  return vars(
    Object.fromEntries(
      Object.entries(colors).map(([name, hex]) => [`--${name}`, channels(hex)]),
    ) as Record<`--${string}`, string>,
  );
}

const THEMES = {
  light: { dark: false, colors: LIGHT, variables: cssVariables(LIGHT) },
  dark: { dark: true, colors: DARK, variables: cssVariables(DARK) },
};

export type Theme = (typeof THEMES)["light"];

/** Set by a part of the screen that is dark whatever the device says (`components/stage.tsx`). */
export const StageTheme = createContext<Theme | null>(null);
export const DARK_THEME: Theme = THEMES.dark;

/** The theme of the device's appearance setting (or of the dark stage around the component). */
export function useTheme(): Theme {
  const stage = useContext(StageTheme);
  const device = useColorScheme() === "dark" ? THEMES.dark : THEMES.light;
  return stage ?? device;
}

/** Ink on the bright fills (lime, sky, sun, mint) in both themes, as on the web. */
export const INK_ON_BRIGHT = LIGHT.ink;

/** "sticker": ink outline + hard shadow, for standalone objects (the web's `.sticker`). */
export function sticker(colors: Colors, size: "sm" | "md" = "md") {
  const depth = size === "sm" ? 2 : 4;
  return {
    borderWidth: 2,
    borderColor: colors.ink,
    boxShadow: `0 ${depth}px 0 0 ${colors.shadow}`,
  } as const;
}
