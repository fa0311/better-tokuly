import type { QueryClient } from "@tanstack/react-query";
import { applyNotesEvent, isNoteNotification, putRefreshedNote } from "@tokuly/core/notes";
import type { ServerFrame } from "@tokuly/realtime";
import { markNoteDeleted, noteClient } from "./api";
import { hintTimelines } from "./timeline-hints";

/** Puts a note as the server shows it now into every cached list (keeping its counters, which move on their own). */
export async function refreshNote(queryClient: QueryClient, noteId: string): Promise<void> {
  try {
    const fresh = (await noteClient().getNote({ noteId })).note;
    if (fresh) putRefreshedNote(queryClient, fresh);
  } catch {
    // Gone or hidden meanwhile: the other pushes say so.
  }
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
