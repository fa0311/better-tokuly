import { colors, overlay, popDepth, radius } from "@tokuly/core/tokens";

/** `#rrggbb` and an opacity → `rgb(r g b / a)`. */
function see(hex: string, opacity: number): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `rgb(${(value >> 16) & 255} ${(value >> 8) & 255} ${value & 255} / ${opacity})`;
}

const rem = (pixels: number) => `${pixels / 16}rem`;

/**
 * `src/tokens.css`, from `@tokuly/core/tokens`: the colours of both themes
 * as variables on the root, and their Tailwind names (`bg-surface`,
 * `text-ink-soft`, `rounded-card`, `shadow-pop`). `pnpm --filter @tokuly/web
 * tokens` writes it; a test fails while it is out of date.
 */
export function tokensCss(): string {
  const names = Object.keys(colors.light) as (keyof typeof colors.light)[];
  const changedInDark = names.filter((name) => colors.dark[name] !== colors.light[name]);
  const lines = [
    "/* Generated from @tokuly/core/tokens by `pnpm --filter @tokuly/web tokens`: do not edit. */",
    "",
    ":root {",
    ...names.map((name) => `  --${name}: ${colors.light[name]};`),
    `  --overlay: ${see(overlay.light.color, overlay.light.opacity)};`,
    "}",
    "",
    ':root[data-theme="dark"] {',
    ...changedInDark.map((name) => `  --${name}: ${colors.dark[name]};`),
    `  --overlay: ${see(overlay.dark.color, overlay.dark.opacity)};`,
    "}",
    "",
    "@theme inline {",
    // The shadow colour is used through `shadow-pop` only.
    ...[...names.filter((name) => name !== "shadow"), "overlay"].map(
      (name) => `  --color-${name}: var(--${name});`,
    ),
    "",
    ...Object.entries(radius).map(([name, pixels]) => `  --radius-${name}: ${rem(pixels)};`),
    "",
    ...Object.entries(popDepth).map(
      ([size, pixels]) =>
        `  --shadow-pop${size === "md" ? "" : `-${size}`}: 0 ${pixels}px 0 0 var(--shadow);`,
    ),
    "}",
    "",
  ];
  return lines.join("\n");
}
