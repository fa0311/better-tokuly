/**
 * "Something new may belong on top of a timeline": timelines on screen
 * listen and look. The realtime connection has no event for a friend's new
 * note, so the hints come from what it does push — a notification about a
 * note (a reply, a quote, a mention, a renote), a reply / quote / renote
 * count going up on a note on screen, the connection opening — and from
 * the page becoming visible again.
 */
const hintListeners = new Set<() => void>();

export function onTimelineHint(listener: () => void): () => void {
  hintListeners.add(listener);
  return () => hintListeners.delete(listener);
}

export function hintTimelines(): void {
  for (const listener of hintListeners) listener();
}
