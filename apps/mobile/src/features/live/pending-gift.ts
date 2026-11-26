import type { GiftAttempt } from "@tokuly/core/live";
import { readJson, writeJson, writeSecure } from "#/platform/secure-storage";

/**
 * The gift of an account whose answer has not come: the network went away,
 * or the app was closed while it was on its way. It may have been paid, so
 * it is written down before it is sent, by user ID, and stays until the
 * server answers: sending the same gift on that stream again, also after
 * the app was started anew, repeats it under its key and nothing is paid
 * twice. One per account: a different gift sent meanwhile takes its place.
 * All accounts' records are one secure-storage entry (`PENDING_GIFTS_KEY`),
 * which the first launch of a new install forgets with the accounts: the
 * iOS keychain outlives an uninstall.
 */
interface PendingGift {
  streamId: string;
  key: string;
  /** A decimal number: JSON has no bigint. */
  coins: string;
  message: string;
}

/** user ID → that account's unanswered gift. */
export const PENDING_GIFTS_KEY = "tokuly.live.gifts";
type PendingGifts = Record<string, PendingGift>;

const readAll = () => readJson<PendingGifts>(PENDING_GIFTS_KEY, {});

/** The gift `userId` sent on a stream that is still unanswered, if any. */
export async function readPendingGift(
  userId: string,
  streamId: string,
): Promise<GiftAttempt | null> {
  const pending = (await readAll())[userId];
  if (pending?.streamId !== streamId) return null;
  return { key: pending.key, coins: BigInt(pending.coins), message: pending.message };
}

/** Writes an attempt down; it is sent only after this. */
export async function keepPendingGift(
  userId: string,
  streamId: string,
  attempt: GiftAttempt,
): Promise<void> {
  const all = await readAll();
  all[userId] = {
    streamId,
    key: attempt.key,
    coins: String(attempt.coins),
    message: attempt.message,
  };
  await writeJson(PENDING_GIFTS_KEY, all);
}

/** The server answered an attempt (the gift, or a refusal): there is nothing left to repeat. */
export async function settlePendingGift(userId: string, attempt: GiftAttempt): Promise<void> {
  const all = await readAll();
  if (all[userId]?.key !== attempt.key) return;
  delete all[userId];
  await (Object.keys(all).length > 0
    ? writeJson(PENDING_GIFTS_KEY, all)
    : writeSecure(PENDING_GIFTS_KEY, null));
}
