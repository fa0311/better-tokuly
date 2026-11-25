/**
 * The web app's design tokens ("sticker pop") as Tailwind names. The values
 * are CSS variables set on the root view from `src/lib/theme.ts`, so both
 * themes share one set of class names: `bg-surface`, `text-ink-soft`, ...
 */
const token = (name) => `rgb(var(--${name}) / <alpha-value>)`;

const COLORS = [
  "bg",
  "surface",
  "surface-2",
  "surface-3",
  "ink",
  "ink-soft",
  "ink-faint",
  "line",
  "brand",
  "brand-strong",
  "brand-soft",
  "on-brand",
  "lime",
  "pink",
  "sky",
  "sun",
  "coral",
  "mint",
  "danger",
  "success",
  "live",
];

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./app/**/*.{ts,tsx}", "./src/**/*.{ts,tsx}"],
  presets: [require("nativewind/preset")],
  theme: {
    extend: {
      colors: Object.fromEntries(COLORS.map((name) => [name, token(name)])),
      borderRadius: { card: "24px", control: "16px" },
    },
  },
  plugins: [],
};
