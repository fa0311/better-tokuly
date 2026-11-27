import { nextSequence } from "@tokuly/core/map";
import { readSecure, writeSecure } from "#/platform/secure-storage";

/**
 * The running count (`nextSequence`) of an account on this device. Per
 * account, as its choice is: the count says nothing about who else is
 * signed in here.
 */
const sequenceKey = (userId: string) => `tokuly.map.sequence.${userId}`;

interface Count {
  latest: number;
  loaded: Promise<void>;
  saved: Promise<void>;
}

const counts = new Map<string, Count>();

function countOf(userId: string): Count {
  let count = counts.get(userId);
  if (!count) {
    const fresh: Count = { latest: 0, loaded: Promise.resolve(), saved: Promise.resolve() };
    fresh.loaded = readSecure(sequenceKey(userId)).then(
      (stored) => {
        fresh.latest = Math.max(fresh.latest, Number(stored) || 0);
      },
      () => undefined,
    );
    counts.set(userId, fresh);
    count = fresh;
  }
  return count;
}

/** Reads where the count of `userId` stood, once per start of the app (or of the background task alone). */
export function loadSequence(userId: string): Promise<void> {
  return countOf(userId).loaded;
}

/**
 * The next number of `userId`, for a sample or a stop, without a wait:
 * taken right where the last check that sharing is on was made, so a stop
 * gets a number above every sample sent before it, whatever the clock does.
 * It is written down (always the latest, in order) for the next start.
 */
export function takeSequence(userId: string): bigint {
  const count = countOf(userId);
  const latest = nextSequence(count.latest, Date.now());
  count.latest = latest;
  count.saved = count.saved
    .then(() => writeSecure(sequenceKey(userId), String(latest)))
    .catch(() => undefined);
  return BigInt(latest);
}
