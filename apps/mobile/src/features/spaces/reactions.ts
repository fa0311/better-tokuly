import { randomUUID } from "expo-crypto";

/** An emoji someone sent into a room. Bursts do not say who sent them. */
export interface Burst {
  spaceId: string;
  emoji: string;
}

/** Nonces of the bursts this phone sent and has not heard back yet. */
const sentNonces = new Set<string>();
const MAX_SENT_NONCES = 64;

/**
 * A fresh tag for a burst about to be sent, so the copy that comes back over
 * realtime is recognised and not drawn twice. Random for every burst:
 * everyone in the room sees it.
 */
export function newBurstNonce(): string {
  const nonce = randomUUID();
  sentNonces.add(nonce);
  if (sentNonces.size > MAX_SENT_NONCES) {
    const oldest = sentNonces.values().next().value;
    if (oldest !== undefined) sentNonces.delete(oldest);
  }
  return nonce;
}

/** Whether a pushed burst is one this phone sent (and drew when it was tapped). */
export function isOwnBurst(nonce: string): boolean {
  return nonce !== "" && sentNonces.delete(nonce);
}

const listeners = new Set<(burst: Burst) => void>();

/** Bursts are stored nowhere: whoever draws them listens here. */
export function onBurst(listener: (burst: Burst) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function emitBurst(burst: Burst): void {
  for (const listener of listeners) listener(burst);
}
