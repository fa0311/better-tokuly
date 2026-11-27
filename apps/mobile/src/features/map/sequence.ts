import { nextSequence } from "@tokuly/core/map";
import { readSecure, writeSecure } from "#/platform/secure-storage";

/** This device's running count (`nextSequence`), one for every account signed in here. */
const SEQUENCE_KEY = "tokuly.map.sequence";

let latest = 0;
let loading: Promise<void> | null = null;
let saving: Promise<void> = Promise.resolve();

/** Reads where the count stood, once per start of the app (or of the background task alone). */
export function loadSequence(): Promise<void> {
  loading ??= readSecure(SEQUENCE_KEY).then(
    (stored) => {
      latest = Math.max(latest, Number(stored) || 0);
    },
    () => undefined,
  );
  return loading;
}

/**
 * The next number, for a sample or a stop, without a wait: taken right
 * where the last check that sharing is on was made, so a stop gets a
 * number above every sample sent before it, whatever the clock does. It is
 * written down (always the latest, in order) for the next start.
 */
export function takeSequence(): bigint {
  latest = nextSequence(latest, Date.now());
  saving = saving.then(() => writeSecure(SEQUENCE_KEY, String(latest))).catch(() => undefined);
  return BigInt(latest);
}
