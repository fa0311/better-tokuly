/**
 * The accent colour of the account, applied on top of the theme: the brand
 * tokens of `styles.css` are redefined for both themes in a small stylesheet
 * of our own, so every `bg-brand` / `text-brand` in the app follows.
 */

/** The colour the design ships with; choosing it removes the override. */
export const DEFAULT_ACCENT = "#7c5cff";

/** Colours offered in the settings; all are readable as text on a light surface. Names are `appearance.accent.colors.<id>`. */
export const ACCENTS = [
  { id: "violet", color: DEFAULT_ACCENT },
  { id: "pink", color: "#ec3f8c" },
  { id: "coral", color: "#f0553a" },
  { id: "orange", color: "#e07a00" },
  { id: "green", color: "#12a36b" },
  { id: "teal", color: "#0e9aa7" },
  { id: "blue", color: "#2f7df6" },
  { id: "ink", color: "#4a4266" },
] as const;

const ACCENT_KEY = "tokuly.accent";
const STYLE_ID = "tokuly-accent";

type Rgb = [number, number, number];

export function isAccentColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value);
}

function parse(hex: string): Rgb {
  return [1, 3, 5].map((start) => Number.parseInt(hex.slice(start, start + 2), 16)) as Rgb;
}

function format(rgb: Rgb): string {
  return `#${rgb.map((channel) => Math.round(channel).toString(16).padStart(2, "0")).join("")}`;
}

/** `amount` of `b` mixed into `a` (0 = all `a`, 1 = all `b`). */
function mix(a: Rgb, b: Rgb, amount: number): Rgb {
  return a.map((channel, index) => channel + ((b[index] ?? 0) - channel) * amount) as Rgb;
}

/** WCAG relative luminance, 0 (black) … 1 (white). */
function luminance(rgb: Rgb): number {
  const [r, g, b] = rgb.map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  }) as Rgb;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];
const INK = "#17122b";
/** The dark theme's page background. */
const NIGHT: Rgb = [13, 11, 20];

export interface AccentTokens {
  brand: string;
  brandStrong: string;
  brandSoft: string;
  onBrand: string;
}

/**
 * The brand tokens for an accent colour, per theme. Light: the colour itself
 * with white or ink text on it, whichever reads better. Dark: a lighter tint
 * (as the shipped palette does) with dark text on it.
 */
export function accentTokens(color: string): { light: AccentTokens; dark: AccentTokens } {
  const accent = parse(color);
  const tint = mix(accent, WHITE, 0.25);
  return {
    light: {
      brand: format(accent),
      brandStrong: format(mix(accent, BLACK, 0.22)),
      brandSoft: format(mix(accent, WHITE, 0.84)),
      onBrand: luminance(accent) > 0.4 ? INK : "#ffffff",
    },
    dark: {
      brand: format(tint),
      brandStrong: format(mix(accent, WHITE, 0.45)),
      brandSoft: format(mix(accent, NIGHT, 0.72)),
      onBrand: luminance(tint) > 0.2 ? "#120c2b" : "#ffffff",
    },
  };
}

function rule(selector: string, tokens: AccentTokens): string {
  return `${selector}{--brand:${tokens.brand};--brand-strong:${tokens.brandStrong};--brand-soft:${tokens.brandSoft};--on-brand:${tokens.onBrand};}`;
}

/** Applies an accent colour to the whole app and remembers it for the next visit. */
export function applyAccent(color: string): void {
  const accent = isAccentColor(color) ? color.toLowerCase() : DEFAULT_ACCENT;
  const existing = document.getElementById(STYLE_ID);
  if (accent === DEFAULT_ACCENT) {
    existing?.remove();
    window.localStorage.removeItem(ACCENT_KEY);
    return;
  }
  const tokens = accentTokens(accent);
  const style = existing ?? document.head.appendChild(Object.assign(document.createElement("style"), { id: STYLE_ID }));
  style.textContent = rule(":root", tokens.light) + rule(':root[data-theme="dark"]', tokens.dark);
  window.localStorage.setItem(ACCENT_KEY, accent);
}

/** The accent applied on the last visit (the default when none was chosen). */
export function storedAccent(): string {
  const value = window.localStorage.getItem(ACCENT_KEY);
  return value && isAccentColor(value) ? value : DEFAULT_ACCENT;
}
