/**
 * Theme and ドパガキ度, applied as attributes on <html> (see styles.css).
 * The last applied values are cached in localStorage so the next visit paints
 * correctly before any network request; `BOOT_SCRIPT` reads that cache.
 */
export type ThemeSetting = "system" | "light" | "dark";

const THEME_KEY = "tokuly.theme";
const DOPA_KEY = "tokuly.dopa";

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
export const BOOT_SCRIPT = `(function(){try{var t=localStorage.getItem("${THEME_KEY}");var d=localStorage.getItem("${DOPA_KEY}")||"3";var dark=t==="dark"||(t!=="light"&&matchMedia("(prefers-color-scheme: dark)").matches);var r=document.documentElement;r.dataset.theme=dark?"dark":"light";r.dataset.dopa=d;}catch(e){}})();`;
