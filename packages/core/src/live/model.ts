import { live, toMillis } from "@tokuly/api";
import { formatCoins } from "../wallet/format.js";

/**
 * The parts of watching a live stream that are plain arithmetic: which tier
 * a gift lands in, how a chat list takes pushes, whether the viewer may
 * write, how a poll is shown. No UI, no network.
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

/**
 * What a stream offers the caller around gifts: `send` (the gift button) for
 * viewers of a stream on air that takes gifts; `age-restricted` for its owner
 * while gifts are on but viewers cannot send any, because gifts are for
 * streamers of 18 and over; `none` otherwise. Viewers are never told why a
 * stream takes no gifts.
 */
export function giftEntry(stream: live.Stream): "send" | "age-restricted" | "none" {
  const status = stream.summary?.status;
  if (!stream.giftsEnabled) return "none";
  if (stream.viewer?.isOwner) {
    const open = status === live.StreamStatus.UPCOMING || status === live.StreamStatus.LIVE;
    return open && stream.studio?.giftsAgeRestricted ? "age-restricted" : "none";
  }
  return status === live.StreamStatus.LIVE ? "send" : "none";
}

export function tierByLevel(tiers: live.GiftTier[], level: number): live.GiftTier | undefined {
  return tiers.find((tier) => tier.level === level);
}

/** Reads what someone typed as a coin amount; `null` when it is not a whole positive number. */
export function parseCoins(text: string): bigint | null {
  const digits = text.replace(/[,\s，]/g, "");
  if (!/^\d{1,9}$/.test(digits)) return null;
  const amount = BigInt(digits);
  return amount > 0n ? amount : null;
}

/** A live gift amount; BigInt balances stay exact while numbers retain locale formatting. */
export function exactCoins(coins: bigint | number, language: string): string {
  return typeof coins === "bigint"
    ? formatCoins(coins)
    : new Intl.NumberFormat(language, { maximumFractionDigits: 0 }).format(coins);
}

/**
 * A key the server accepts for one gift attempt (8-64 of letters, digits,
 * `-`, `_`), from `random` (a UUID of the platform's own generator).
 */
export function newIdempotencyKey(random: () => string): string {
  return `gift-${random()}`.replace(/[^A-Za-z0-9_-]/g, "").slice(0, 64);
}

/** One gift attempt: the key the server deduplicates by, and what it was sent with. */
export interface GiftAttempt {
  key: string;
  coins: bigint;
  message: string;
}

/**
 * The attempt a send goes out under. Repeating exactly the gift whose answer
 * was lost keeps its key, so the server finishes that one and nothing is paid
 * twice; any other amount or message is a new gift with a new key (the
 * server would otherwise answer with the first gift).
 */
export function giftAttempt(
  previous: GiftAttempt | null,
  coins: bigint,
  message: string,
  newKey: () => string,
): GiftAttempt {
  if (previous && previous.coins === coins && previous.message === message) return previous;
  return { key: newKey(), coins, message };
}

/** Whether the gift the server answered with is the one that was asked for. */
export function sameGift(attempt: GiftAttempt, gift: { coins: bigint; message: string }): boolean {
  return gift.coins === attempt.coins && gift.message === attempt.message;
}

// ───────────────────────────── Chat ─────────────────────────────

/** How many lines a chat keeps in memory; older ones scroll away for good. */
export const CHAT_LIMIT = 500;

/** The longest chat line the server takes. */
export const CHAT_MAX_LENGTH = 200;

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

/**
 * Why the viewer cannot write in the chat right now, as the server last said
 * (one of the SendChatMessage reasons); empty when they can. A timeout that
 * ran out by `nowMs` no longer counts. Guests (`viewer` unset) are not
 * blocked here: they are asked to sign in instead.
 */
export function chatBlockReason(viewer: live.StreamViewerState | undefined, nowMs: number): string {
  if (!viewer || viewer.canChat) return "";
  if (viewer.chatBlockReason === "CHAT_TIMED_OUT") {
    const until = viewer.chatBlockedUntil ? toMillis(viewer.chatBlockedUntil) : 0;
    if (until <= nowMs) return "";
  }
  return viewer.chatBlockReason;
}

/** Who a chat line is from, as text: the channel for the streamer, the person otherwise. */
export function authorName(message: live.ChatMessage): string {
  return message.author?.channel?.name || message.author?.user?.displayName || "";
}

/** The `@handle` a reply to this line starts with; empty for the streamer (a channel has no account to mention). */
export function mentionOf(message: live.ChatMessage): string {
  const user = message.author?.user;
  return user && !user.deleted ? `@${user.handle} ` : "";
}

/**
 * A chat line cut into its plain text and the `@handle`s in it, which are
 * marked (`mine` for the reader's own `selfHandle`, lower case).
 */
export function chatTextParts(
  text: string,
  selfHandle?: string,
): { text: string; mention: boolean; mine: boolean }[] {
  return text
    .split(/(@[A-Za-z0-9_-]{3,24})/g)
    .map((part, index) => {
      const mention = index % 2 === 1;
      const mine =
        mention && selfHandle !== undefined && part.slice(1).toLowerCase() === selfHandle;
      return { text: part, mention, mine };
    })
    .filter((part) => part.text !== "");
}

// ───────────────────────────── Polls ─────────────────────────────

/** How long a closed poll keeps showing its result to viewers. */
export const POLL_RESULT_SECONDS = 20;

/** Whether a poll still belongs on screen: open, or closed a moment ago. */
export function pollVisible(poll: live.Poll | undefined, nowMs: number): poll is live.Poll {
  if (!poll) return false;
  if (!poll.endedAt) return true;
  return nowMs - toMillis(poll.endedAt) < POLL_RESULT_SECONDS * 1000;
}

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
