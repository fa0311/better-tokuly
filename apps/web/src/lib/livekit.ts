import type { Room } from "livekit-client";
import type { StoreApi } from "zustand";

/**
 * What chat calls and audio rooms (spaces) share about holding one LiveKit
 * room for the whole app: the client loaded on first use, the store of the
 * session in progress, the hidden element voices play through, and the tap
 * that lets the browser play sound.
 */

/** The LiveKit client, loaded the first time a call or a room needs it: it is large. */
export function loadLiveKit(): Promise<typeof import("livekit-client")> {
  return import("livekit-client");
}

/** A store holding at most one session in progress, whose views redraw when `revision` moves. */
type SessionStore = StoreApi<{ active: { revision: number } | null }>;

/**
 * `update` (merge a change into the session in progress) and `touch` (tell
 * its views that the room's participants or tracks changed) for `store`.
 * Both do nothing once the session is over.
 */
export function sessionWriters<T extends { active: { revision: number } | null }>(
  store: StoreApi<T>,
): { update(change: Partial<NonNullable<T["active"]>>): void; touch(): void } {
  const sessions = store as unknown as SessionStore;
  return {
    update(change) {
      sessions.setState((state) =>
        state.active ? { active: { ...state.active, ...change } } : state,
      );
    },
    touch() {
      sessions.setState((state) =>
        state.active ? { active: { ...state.active, revision: state.active.revision + 1 } } : state,
      );
    },
  };
}

/**
 * The hidden element with `id` that remote voices play through, created on
 * first use. It lives outside React, so moving around the app never
 * interrupts them.
 */
export function audioSink(id: string): HTMLElement {
  let element = document.getElementById(id);
  if (!element) {
    element = document.createElement("div");
    element.id = id;
    element.hidden = true;
    document.body.appendChild(element);
  }
  return element;
}

/** Silences and empties the element `audioSink(id)` made, if there is one. */
export function clearAudioSink(id: string): void {
  document.getElementById(id)?.replaceChildren();
}

/**
 * After a tap: lets the browser play the room's sound. `room` is read again
 * afterwards (the session may have changed meanwhile). Resolves to whether
 * playback is still blocked.
 */
export async function startRoomAudio(room: () => Room | null): Promise<boolean> {
  await room()
    ?.startAudio()
    .catch(() => {});
  const current = room();
  return current ? !current.canPlaybackAudio : false;
}
