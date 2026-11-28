/**
 * Theme and ドパガキ度, applied as attributes on <html> (see styles.css), and
 * the account's colours, a stylesheet of token variables
 * (`features/settings/appearance/colors.ts`). The last applied values are
 * cached in localStorage so the next visit paints correctly before any
 * network request; `BOOT_SCRIPT` reads that cache.
 */
export type ThemeSetting = "system" | "light" | "dark";

const THEME_KEY = "tokuly.theme";
const DOPA_KEY = "tokuly.dopa";
export const COLORS_KEY = "tokuly.colors";
export const COLORS_STYLE_ID = "tokuly-colors";

/**
 * What the colours stylesheet may hold: token variables set to #rrggbb, on
 * the root of each theme (doubled, so it wins over `tokens.css` wherever it
 * sits in <head>). Anything else found in storage is not applied.
 */
export const COLORS_CSS =
  /^(?::root:root(?:\[data-theme="dark"\])?\{(?:--[a-z0-9-]+:#[0-9a-f]{6};)+\})+$/;

function resolveTheme(setting: ThemeSetting): "light" | "dark" {
  if (setting !== "system") return setting;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyAppearance(theme: ThemeSetting, dopamineLevel: number): void {
  const root = document.documentElement;
  root.dataset.theme = resolveTheme(theme);
  root.dataset.dopa = String(Math.min(5, Math.max(0, Math.round(dopamineLevel))));
  window.localStorage.setItem(THEME_KEY, theme);
  window.localStorage.setItem(DOPA_KEY, root.dataset.dopa);
}

export function storedTheme(): ThemeSetting {
  const value = window.localStorage.getItem(THEME_KEY);
  return value === "light" || value === "dark" ? value : "system";
}

export function storedDopamineLevel(): number {
  const value = Number(window.localStorage.getItem(DOPA_KEY) ?? "3");
  return Number.isFinite(value) ? value : 3;
}

/** How lively animations should be right now: 0 (off) … 1.6. */
export function dopamineFactor(): number {
  if (typeof window === "undefined") return 1;
  const value = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--dopa"),
  );
  return Number.isFinite(value) ? value : 1;
}

/** Inline script for <head>: sets the attributes before first paint. */
export const BOOT_SCRIPT = `(function(){try{var t=localStorage.getItem("${THEME_KEY}");var d=localStorage.getItem("${DOPA_KEY}")||"3";var dark=t==="dark"||(t!=="light"&&matchMedia("(prefers-color-scheme: dark)").matches);var r=document.documentElement;r.dataset.theme=dark?"dark":"light";r.dataset.dopa=d;var c=localStorage.getItem("${COLORS_KEY}");if(c&&${COLORS_CSS}.test(c)){var s=document.createElement("style");s.id="${COLORS_STYLE_ID}";s.textContent=c;document.head.appendChild(s);}}catch(e){}})();`;
