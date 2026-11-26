import type { GiftAttempt } from "@tokuly/core/live";

/**
 * The gift of an account whose answer has not come (the network went away,
 * the tab was closed while it was on its way). It may have been paid, so it
 * is written down before it is sent, by user ID, and stays until the server
 * answers: sending the same gift on that stream again — after going back,
 * closing the dialog or loading the page anew — repeats it under its key and
 * nothing is paid twice. One per account: a different gift sent meanwhile
 * takes its place. (The phone keeps the same record in its secure storage.)
 */
interface PendingGift {
  streamId: string;
  key: string;
  /** A decimal number: JSON has no bigint. */
  coins: string;
  message: string;
}

/** Where the record lives; `null` where the browser keeps nothing (private mode, a server render). */
export type GiftStore = Pick<Storage, "getItem" | "setItem" | "removeItem"> | null;

function browserStore(): GiftStore {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

const storageKey = (userId: string) => `tokuly.live.gift.${userId}`;

function read(store: GiftStore, userId: string): PendingGift | null {
  try {
    const text = store?.getItem(storageKey(userId));
    return text ? (JSON.parse(text) as PendingGift) : null;
  } catch {
    return null;
  }
}

/** The gift `userId` sent on a stream that is still unanswered, if any. */
export function readPendingGift(
  userId: string,
  streamId: string,
  store: GiftStore = browserStore(),
): GiftAttempt | null {
  const pending = read(store, userId);
  if (pending?.streamId !== streamId) return null;
  return { key: pending.key, coins: BigInt(pending.coins), message: pending.message };
}

/** Writes an attempt down; it is sent only after this. */
export function keepPendingGift(
  userId: string,
  streamId: string,
  attempt: GiftAttempt,
  store: GiftStore = browserStore(),
): void {
  const pending: PendingGift = {
    streamId,
    key: attempt.key,
    coins: String(attempt.coins),
    message: attempt.message,
  };
  try {
    store?.setItem(storageKey(userId), JSON.stringify(pending));
  } catch {
    // Full or blocked: the attempt is still kept for as long as the page is open.
  }
}

/** The server answered an attempt (the gift, or a refusal): there is nothing left to repeat. */
export function settlePendingGift(
  userId: string,
  attempt: GiftAttempt,
  store: GiftStore = browserStore(),
): void {
  if (read(store, userId)?.key !== attempt.key) return;
  try {
    store?.removeItem(storageKey(userId));
  } catch {
    // Nothing to do: a stale record only ever repeats a gift under its own key.
  }
}
