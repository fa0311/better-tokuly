import type { live } from "@tokuly/api";

/**
 * The parts of live streaming that are plain arithmetic: which tier a gift
 * lands in, how a chat list takes pushes, where a clip may start. No React,
 * no network.
 */

// ───────────────────────────── Gifts ─────────────────────────────

/** The tier a gift of `coins` belongs to: the highest one whose minimum it reaches. */
export function tierFor(tiers: live.GiftTier[], coins: bigint | number): live.GiftTier | undefined {
  const amount = BigInt(coins);
  let found: live.GiftTier | undefined;
  for (const tier of tiers) {
    if (tier.minCoins <= amount && (!found || tier.minCoins > found.minCoins)) found = tier;
  }
  return found;
}

export function tierByLevel(tiers: live.GiftTier[], level: number): live.GiftTier | undefined {
  return tiers.find((tier) => tier.level === level);
}

/** CSS for a tier's highlight: its flat colour, or the gradient of the top tier. */
export function tierStyle(tier: live.GiftTier | undefined): { background: string; color: string } {
  if (!tier) return { background: "var(--surface-3)", color: "var(--ink)" };
  const background =
    tier.gradientColors.length > 1
      ? `linear-gradient(100deg, ${tier.gradientColors.join(", ")})`
      : tier.backgroundColor;
  return { background, color: tier.textColor };
}

/** Reads what someone typed as a coin amount; `null` when it is not a whole positive number. */
export function parseCoins(text: string): bigint | null {
  const digits = text.replace(/[,\s，]/g, "");
  if (!/^\d{1,9}$/.test(digits)) return null;
  const amount = BigInt(digits);
  return amount > 0n ? amount : null;
}

/** A key the server accepts for one gift attempt (8-64 of letters, digits, `-`, `_`). */
export function newIdempotencyKey(random: () => string = () => crypto.randomUUID()): string {
  return `gift-${random()}`.replace(/[^A-Za-z0-9_-]/g, "").slice(0, 64);
}

// ───────────────────────────── Chat ─────────────────────────────

/** How many lines a chat keeps in memory; older ones scroll away for good. */
export const CHAT_LIMIT = 500;

/**
 * Adds lines to a chat that is ordered oldest first. A line that is already
 * there (the sender's own answer and its push) is replaced, not repeated.
 */
export function addChatMessages(
  current: live.ChatMessage[],
  incoming: live.ChatMessage[],
  limit = CHAT_LIMIT,
): live.ChatMessage[] {
  if (incoming.length === 0) return current;
  const byId = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  const merged = [...byId.values()].sort(
    (a, b) =>
      Number(a.offsetMs - b.offsetMs) || createdMs(a) - createdMs(b) || (a.id < b.id ? -1 : 1),
  );
  return merged.length > limit ? merged.slice(merged.length - limit) : merged;
}

function createdMs(message: live.ChatMessage): number {
  const at = message.createdAt;
  return at ? Number(at.seconds) * 1000 + at.nanos / 1e6 : 0;
}

export function removeChatMessage(
  current: live.ChatMessage[],
  messageId: string,
): live.ChatMessage[] {
  return current.some((message) => message.id === messageId)
    ? current.filter((message) => message.id !== messageId)
    : current;
}

/** Drops every line a person wrote (a ban that clears their messages). */
export function removeChatUser(current: live.ChatMessage[], userId: string): live.ChatMessage[] {
  return current.some((message) => message.author?.user?.id === userId)
    ? current.filter((message) => message.author?.user?.id !== userId)
    : current;
}

/** Gift lines whose highlight has not run out at `nowMs`, biggest first. */
export function activeHighlights(gifts: live.ChatMessage[], nowMs: number): live.ChatMessage[] {
  return gifts
    .filter((message) => {
      const until = message.gift?.highlightedUntil;
      return until !== undefined && Number(until.seconds) * 1000 > nowMs;
    })
    .sort((a, b) => Number((b.gift?.coins ?? 0n) - (a.gift?.coins ?? 0n)));
}

/** Seconds a viewer still waits under slow mode after sending at `sentAtMs`. */
export function slowModeWait(slowSeconds: number, sentAtMs: number | null, nowMs: number): number {
  if (slowSeconds <= 0 || sentAtMs === null) return 0;
  return Math.max(0, Math.ceil((sentAtMs + slowSeconds * 1000 - nowMs) / 1000));
}

// ───────────────────────────── Polls ─────────────────────────────

/** Each option's share of the votes in whole percent (0 when nobody voted). */
export function pollShares(poll: live.Poll): number[] {
  const total = Number(poll.totalVotes);
  return poll.options.map((option) =>
    total > 0 ? Math.round((Number(option.voteCount) / total) * 100) : 0,
  );
}

/**
 * A pushed poll never knows the viewer's vote (`viewer_choice` is 0 in
 * pushes): keep the one already known when it is the same poll.
 */
export function mergePoll(current: live.Poll | undefined, pushed: live.Poll): live.Poll {
  if (current?.id === pushed.id && pushed.viewerChoice === 0 && current.viewerChoice !== 0)
    return { ...pushed, viewerChoice: current.viewerChoice };
  return pushed;
}

// ───────────────────────────── Time ─────────────────────────────

/** What is left until a moment, as the parts a countdown shows. */
export function countdownParts(targetMs: number, nowMs: number) {
  const total = Math.max(0, Math.floor((targetMs - nowMs) / 1000));
  return {
    total,
    days: Math.floor(total / 86_400),
    hours: Math.floor((total % 86_400) / 3600),
    minutes: Math.floor((total % 3600) / 60),
    seconds: total % 60,
  };
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
