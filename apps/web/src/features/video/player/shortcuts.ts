/** What a key press asks of the player. */
export type PlayerCommand =
  | { type: "toggle" }
  | { type: "seekBy"; seconds: number }
  /** 0…1 of the video's length (the digit keys). */
  | { type: "seekToFraction"; fraction: number }
  | { type: "volumeBy"; delta: number }
  | { type: "mute" }
  | { type: "fullscreen" }
  | { type: "theatre" }
  | { type: "captions" }
  | { type: "rateBy"; delta: number }
  | { type: "pip" };

/** The part of a keyboard event the shortcuts look at. */
export interface KeyPress {
  key: string;
  code?: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
}

/**
 * The player command for a key, the way people know them from other video
 * sites: space / k play and pause, j / l ten seconds, arrows five seconds and
 * volume, f fullscreen, m mute, t theatre, c captions, i picture-in-picture,
 * digits jump to tenths, `<` / `>` change speed. `null` for anything else —
 * including every combination with Ctrl, Cmd or Alt, which belong to the browser.
 */
export function commandForKey(press: KeyPress): PlayerCommand | null {
  if (press.ctrlKey || press.metaKey || press.altKey) return null;
  switch (press.key) {
    case " ":
    case "k":
    case "K":
      return { type: "toggle" };
    case "j":
    case "J":
      return { type: "seekBy", seconds: -10 };
    case "l":
    case "L":
      return { type: "seekBy", seconds: 10 };
    case "ArrowLeft":
      return { type: "seekBy", seconds: -5 };
    case "ArrowRight":
      return { type: "seekBy", seconds: 5 };
    case "ArrowUp":
      return { type: "volumeBy", delta: 0.05 };
    case "ArrowDown":
      return { type: "volumeBy", delta: -0.05 };
    case "Home":
      return { type: "seekToFraction", fraction: 0 };
    case "End":
      return { type: "seekToFraction", fraction: 1 };
    case "f":
    case "F":
      return { type: "fullscreen" };
    case "m":
    case "M":
      return { type: "mute" };
    case "t":
    case "T":
      return { type: "theatre" };
    case "c":
    case "C":
      return { type: "captions" };
    case "i":
    case "I":
      return { type: "pip" };
    case "<":
      return { type: "rateBy", delta: -0.25 };
    case ">":
      return { type: "rateBy", delta: 0.25 };
    default:
      break;
  }
  // Digits by key or — on layouts where the digit row needs Shift — by position.
  const digit = /^[0-9]$/.test(press.key) ? Number(press.key) : !press.shiftKey && /^(Digit|Numpad)[0-9]$/.test(press.code ?? "") ? Number((press.code ?? "").slice(-1)) : null;
  return digit === null ? null : { type: "seekToFraction", fraction: digit / 10 };
}

const EDITABLE = "input, textarea, select, [contenteditable]:not([contenteditable='false'])";
/** Widgets that use the arrow keys, space or letters themselves. */
const BUSY_WIDGET = "[role='dialog'], [role='menu'], [role='listbox'], [role='tablist'], [role='slider']:not([data-player-scrubber]), [role='radiogroup']";
const ACTIVATABLE = "button, a[href], summary, [role='button'], [role='tab'], [role='switch'], [role='option'], [role='menuitem']";

/**
 * Whether a key pressed with the focus on `target` is the player's to
 * handle: not while typing, not inside a widget with keys of its own, and not
 * the space or Enter that presses a focused button.
 */
export function playerOwnsKey(target: Element | null, press: KeyPress): boolean {
  if (!target) return true;
  if (target.closest(EDITABLE) || target.closest(BUSY_WIDGET)) return false;
  if ((press.key === " " || press.key === "Enter") && target.closest(ACTIVATABLE)) return false;
  return true;
}

/** Playback speeds offered in the menu. */
export const RATES = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2] as const;

/** The next speed up or down from `rate`, staying inside what the menu offers. */
export function stepRate(rate: number, delta: number): number {
  const next = Math.round((rate + delta) * 100) / 100;
  return Math.min(RATES[RATES.length - 1] ?? 2, Math.max(RATES[0] ?? 0.25, next));
}
