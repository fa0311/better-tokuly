import type { QueryClient } from "@tanstack/react-query";
import { applyNotesEvent, isNoteNotification, patchNote } from "@tokuly/core/notes";
import type { ServerFrame } from "@tokuly/realtime";
import { markNoteDeleted, noteClient } from "./api";

/** Puts a note as the server shows it now into every cached list (keeping its counters, which move on their own). */
export async function refreshNote(queryClient: QueryClient, noteId: string): Promise<void> {
  try {
    const fresh = (await noteClient().getNote({ noteId })).note;
    if (fresh)
      patchNote(queryClient, noteId, (note) => ({
        ...fresh,
        counters: note.counters ?? fresh.counters,
      }));
  } catch {
    // Gone or hidden meanwhile: the other pushes say so.
  }
}

/**
 * "Something new may belong on top of a timeline": timelines on screen
 * listen and look. The realtime connection has no event for a friend's new
 * note, so the hints come from what it does push — a notification about a
 * note (a reply, a quote, a mention, a renote), a reply / quote / renote
 * count going up on a note on screen, the connection coming back — and from
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

/** Applies one realtime frame to the cached notes. Returns nothing; unrelated frames are ignored. */
export function applyNotesFrame(queryClient: QueryClient, frame: ServerFrame): void {
  if (frame.body.case === "notify") {
    const event = frame.body.value.event;
    if (event.case === "received" && isNoteNotification(event.value.notification?.kind ?? ""))
      hintTimelines();
    return;
  }
  if (frame.body.case !== "notes") return;
  const effect = applyNotesEvent(queryClient, frame.body.value.event);
  if (effect.hint) hintTimelines();
  if (effect.deleted) markNoteDeleted(effect.deleted);
  // What a reader may see of a poll, or the new words: read the note again.
  if (effect.refresh) void refreshNote(queryClient, effect.refresh);
}
