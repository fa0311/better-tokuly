import type { Subject, ViewerReaction } from "@tokuly/core/engagement";

// Reactions from other people, popping up over a subject's reaction bar as they arrive (the realtime `reaction_added`).

export type Listener = (reaction: ViewerReaction) => void;

export const listeners = new Map<string, Set<Listener>>();
/** What this browser itself just added, by subject and emoji, so its own reaction coming back over the topic does not pop a second time. */
const own = new Map<string, number>();
const OWN_ECHO_MS = 4000;
/** Pops on screen at once per bar; more are skipped (the counts follow anyway). */
export const MAX_POPS = 6;
export const POP_SECONDS = 1.3;

export const keyOf = (type: Subject["type"], uuid: string) => `${type}:${uuid}`;

/** Called by the feature's background component for every `reaction_added`. */
export function emitPop(type: Subject["type"], uuid: string, reaction: ViewerReaction): void {
  const echoKey = `${keyOf(type, uuid)}:${reaction.kind}:${reaction.emoji}`;
  const echo = own.get(echoKey);
  if (echo !== undefined) {
    own.delete(echoKey);
    if (Date.now() - echo < OWN_ECHO_MS) return;
  }
  for (const listener of listeners.get(keyOf(type, uuid)) ?? []) listener(reaction);
}

/** The caller just added this reaction to the subject (its UUID): its echo is not shown. */
export function noteOwnReaction(
  type: Subject["type"],
  uuid: string,
  reaction: ViewerReaction,
): void {
  own.set(`${keyOf(type, uuid)}:${reaction.kind}:${reaction.emoji}`, Date.now());
}
