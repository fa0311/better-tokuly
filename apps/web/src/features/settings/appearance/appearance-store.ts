import { create } from "zustand";
import { applyAppearance, storedDopamineLevel, storedTheme, type ThemeSetting } from "#/lib/appearance";

export interface Appearance {
  theme: ThemeSetting;
  /** ドパガキ度, 0–5. */
  level: number;
}

export const MAX_LEVEL = 5;

export function clampLevel(level: number): number {
  return Math.min(MAX_LEVEL, Math.max(0, Math.round(level)));
}

/**
 * The theme and ドパガキ度 in effect in this browser. `#/lib/appearance`
 * applies and persists them; this store only lets screens follow along when
 * something else (signing in on a new device) changes them.
 */
export const useAppearance = create<Appearance>(() =>
  typeof window === "undefined" ? { theme: "system", level: 3 } : { theme: storedTheme(), level: clampLevel(storedDopamineLevel()) },
);

/** Applies a change at once and keeps it for the next visit. */
export function setAppearance(change: Partial<Appearance>): void {
  const next = { ...useAppearance.getState(), ...change };
  next.level = clampLevel(next.level);
  applyAppearance(next.theme, next.level);
  useAppearance.setState(next);
}

export function isThemeSetting(value: string): value is ThemeSetting {
  return value === "system" || value === "light" || value === "dark";
}
