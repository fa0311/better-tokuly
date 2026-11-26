import type { live } from "@tokuly/api";

/**
 * The parts of live streaming only the web has that are plain arithmetic:
 * the CSS of a gift tier, where a clip may start, how a secret is hidden,
 * an overlay's alert text. The rules every app shares (gifts, chat, polls)
 * are `@tokuly/core/live`. No React, no network.
 */

/** CSS for a tier's highlight: its flat colour, or the gradient of the top tier. */
export function tierStyle(tier: live.GiftTier | undefined): { background: string; color: string } {
  if (!tier) return { background: "var(--surface-3)", color: "var(--ink)" };
  const background =
    tier.gradientColors.length > 1
      ? `linear-gradient(100deg, ${tier.gradientColors.join(", ")})`
      : tier.backgroundColor;
  return { background, color: tier.textColor };
}

// ───────────────────────────── Clips ─────────────────────────────

export const CLIP_MIN_SECONDS = 0.1;
export const CLIP_MAX_SECONDS = 60;

/**
 * Keeps a clip range inside its source and inside the allowed length. The
 * edge being dragged wins: moving the start pulls the end along when the
 * range would get too long, and the other way round.
 */
export function clampClipRange(
  start: number,
  end: number,
  sourceSeconds: number,
  moved: "start" | "end",
): { start: number; end: number } {
  const limit = Math.max(CLIP_MIN_SECONDS, sourceSeconds);
  let from = Math.min(Math.max(0, start), limit - CLIP_MIN_SECONDS);
  let to = Math.min(Math.max(CLIP_MIN_SECONDS, end), limit);
  if (moved === "start") {
    if (to < from + CLIP_MIN_SECONDS) to = Math.min(limit, from + CLIP_MIN_SECONDS);
    if (to - from > CLIP_MAX_SECONDS) to = from + CLIP_MAX_SECONDS;
  } else {
    if (from > to - CLIP_MIN_SECONDS) from = Math.max(0, to - CLIP_MIN_SECONDS);
    if (to - from > CLIP_MAX_SECONDS) from = to - CLIP_MAX_SECONDS;
  }
  return { start: round1(from), end: round1(to) };
}

const round1 = (value: number) => Math.round(value * 10) / 10;

// ───────────────────────────── Secrets ─────────────────────────────

/** A secret as dots, so its length does not leak either. */
export function maskSecret(value: string): string {
  return value ? "•".repeat(12) : "";
}

/** A URL with its secret hidden: scheme and host stay readable, the rest is dots. */
export function maskUrl(value: string): string {
  const match = /^([a-z]+:\/\/[^/?#]+)(.*)$/i.exec(value);
  if (!match) return maskSecret(value);
  return match[2] ? `${match[1]}/${"•".repeat(12)}` : (match[1] ?? "");
}

// ───────────────────────────── Overlay alerts ─────────────────────────────

/** Fills the placeholders of an alert text: `{name}`, `{coins}`, `{message}`, `{count}`. */
export function fillAlertTemplate(
  template: string,
  values: { name: string; coins: string; message: string; count: string },
): string {
  return template.replace(/\{(name|coins|message|count)\}/g, (_, key: keyof typeof values) => {
    return values[key];
  });
}
