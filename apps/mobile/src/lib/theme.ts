import { type Colors, colors, inkOnBright, popDepth } from "@tokuly/core/tokens";
import { vars } from "nativewind";
import { createContext, useContext } from "react";
import { useColorScheme } from "react-native";

/**
 * The design tokens of every Tokuly app ("sticker pop", `@tokuly/core/tokens`).
 * Components use the Tailwind names (`bg-surface`, `text-ink-soft`); native
 * props that take a colour (icons, the status bar, the map) read
 * `useTheme().colors`.
 */
const LIGHT = colors.light;
const DARK = colors.dark;

export type { ColorName, Colors } from "@tokuly/core/tokens";

/** `#rrggbb` → `r g b`, the form `rgb(var(--x) / <alpha-value>)` in tailwind.config.js takes. */
function channels(hex: string): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `${(value >> 16) & 255} ${(value >> 8) & 255} ${value & 255}`;
}

function cssVariables(palette: Colors) {
  return vars(
    Object.fromEntries(
      Object.entries(palette).map(([name, hex]) => [`--${name}`, channels(hex)]),
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
export const INK_ON_BRIGHT = inkOnBright;

/** "sticker": ink outline + hard shadow, for standalone objects (the web's `.sticker`). */
export function sticker(palette: Colors, size: "sm" | "md" = "md") {
  const depth = popDepth[size];
  return {
    borderWidth: 2,
    borderColor: palette.ink,
    boxShadow: `0 ${depth}px 0 0 ${palette.shadow}`,
  } as const;
}
