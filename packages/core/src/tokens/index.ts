/**
 * Tokuly's design tokens ("sticker pop"), as data: rounded shapes outlined
 * in ink with a hard drop shadow, lavender paper, a purple brand with lime,
 * pink, sky and sun next to it. The web's stylesheet is generated from them
 * (`apps/web/src/tokens.css`), the phone's theme reads them
 * (`apps/mobile/src/lib/theme.ts`). Colours are `#rrggbb`; sizes are pixels.
 */
const light = {
  bg: "#f4f1ff",
  surface: "#ffffff",
  "surface-2": "#f0ecfb",
  "surface-3": "#e4def5",
  ink: "#17122b",
  "ink-soft": "#5d5778",
  "ink-faint": "#9993ad",
  line: "#ddd6ee",
  shadow: "#17122b",
  // Deep enough for white text on it, and for it as text on white (4.5:1).
  brand: "#7859f7",
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

export type ColorName = keyof typeof light;
export type Colors = Record<ColorName, string>;

/** The dark theme: the surfaces, the ink and the brand change; the bright fills stay. */
const dark: Colors = {
  ...light,
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

export const colors: { readonly light: Colors; readonly dark: Colors } = { light, dark };

/** What lies over the page behind a dialog: the ink of the theme (light) or black (dark), see-through. */
export const overlay = {
  light: { color: light.ink, opacity: 0.45 },
  dark: { color: "#000000", opacity: 0.6 },
} as const;

/** Corner radii: a card, a control (buttons, fields). */
export const radius = { card: 24, control: 16 } as const;

/** How far the hard shadow of a sticker drops. */
export const popDepth = { sm: 2, md: 4, lg: 6 } as const;

/** Ink on the bright fills (lime, sky, sun, mint), in both themes. */
export const inkOnBright = light.ink;
