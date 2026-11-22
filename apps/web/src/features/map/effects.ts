import type { common, social } from "@tokuly/api";

/**
 * Things that happen on the map for a moment: a reaction flying to a pin, a
 * knock, a buzz, confetti for an arrival. Whoever learns of one (the realtime
 * sync, a button) emits it; the effects layer on the map plays it.
 */
export type MapEffect =
  | {
      kind: "reaction";
      reaction: social.MapReaction;
      direction: "incoming" | "outgoing";
      recap?: boolean;
    }
  | { kind: "knock"; from: common.UserSummary; count: number }
  | { kind: "buzz"; from: common.UserSummary }
  | { kind: "bump"; id: string; with: common.UserSummary; pairCount: number; placeName: string }
  | { kind: "celebrate"; at?: { lat: number; lng: number } };

type Listener = (effect: MapEffect) => void;
const listeners = new Set<Listener>();

export function emitEffect(effect: MapEffect): void {
  for (const listener of listeners) listener(effect);
}

export function onEffect(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
