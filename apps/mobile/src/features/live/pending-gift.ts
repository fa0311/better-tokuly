import type { GiftAttempt } from "@tokuly/core/live";
import { readJson, writeJson, writeSecure } from "#/platform/secure-storage";

/**
 * The gift of an account whose answer has not come: the network went away,
 * or the app was closed while it was on its way. It may have been paid, so
 * it is written down before it is sent, by user ID, and stays until the
 * server answers: sending the same gift on that stream again, also after
 * the app was started anew, repeats it under its key and nothing is paid
 * twice. One per account: a different gift sent meanwhile takes its place.
 */
interface PendingGift {
  streamId: string;
  key: string;
  /** A decimal number: JSON has no bigint. */
  coins: string;
  message: string;
}

const pendingGiftKey = (userId: string) => `tokuly.live.gift.${userId}`;

/** The gift `userId` sent on a stream that is still unanswered, if any. */
export async function readPendingGift(
  userId: string,
  streamId: string,
): Promise<GiftAttempt | null> {
  const pending = await readJson<PendingGift | null>(pendingGiftKey(userId), null);
  if (pending?.streamId !== streamId) return null;
  return { key: pending.key, coins: BigInt(pending.coins), message: pending.message };
}

/** Writes an attempt down; it is sent only after this. */
export function keepPendingGift(
  userId: string,
  streamId: string,
  attempt: GiftAttempt,
): Promise<void> {
  const pending: PendingGift = {
    streamId,
    key: attempt.key,
    coins: String(attempt.coins),
    message: attempt.message,
  };
  return writeJson(pendingGiftKey(userId), pending);
}

/** The server answered an attempt (the gift, or a refusal): there is nothing left to repeat. */
export async function settlePendingGift(userId: string, attempt: GiftAttempt): Promise<void> {
  const key = pendingGiftKey(userId);
  const pending = await readJson<PendingGift | null>(key, null);
  if (pending?.key === attempt.key) await writeSecure(key, null);
}
