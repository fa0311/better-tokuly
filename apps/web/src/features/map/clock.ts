import { useSyncExternalStore } from "react";

/**
 * The server's clock, as far as this device can tell. Timers the server
 * runs (a ghost rule that ends at 18:00) must not end early or late because
 * a phone's clock is a few minutes off: every time the server says what time
 * it is, the difference is remembered and applied.
 */
let offsetMs = 0;

export function setServerTime(serverMs: number): void {
  if (serverMs > 0) offsetMs = serverMs - Date.now();
}

/** Now, by the server's clock. */
export function serverNow(): number {
  return Date.now() + offsetMs;
}

const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  // One timer for every pin, list row and countdown on the screen.
  timer ??= setInterval(() => {
    for (const notify of listeners) notify();
  }, 1000);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

/**
 * The current time (server clock), re-rendering the caller every
 * `intervalMs` so "NOW" / "12分" / "あと52分" stay current. All callers share
 * one timer; each only re-renders at its own pace.
 */
export function useNow(intervalMs: number): number {
  return useSyncExternalStore(
    subscribe,
    () => Math.floor(serverNow() / intervalMs) * intervalMs,
    () => 0,
  );
}
