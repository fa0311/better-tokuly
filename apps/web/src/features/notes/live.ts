import type { QueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import type { ServerFrame } from "@tokuly/realtime";
import {
  markNoteDeleted,
  noteClient,
  notesKeys,
  patchNote,
  type TimelineData,
  withCounters,
} from "./api";

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

/** The counters a note has in any cached timeline, to tell whether a pushed count went up. */
function cachedCounters(queryClient: QueryClient, noteId: string): notes.NoteCounters | undefined {
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notesKeys.timelines })) {
    const data = query.state.data as TimelineData | undefined;
    for (const page of data?.pages ?? []) {
      for (const entry of page.entries) {
        if (entry.note?.id === noteId) return entry.note.counters;
      }
    }
  }
  return undefined;
}

/** Notification kinds that mean a note now belongs in the home or mentions timeline. */
const NOTE_KINDS = new Set(["notes.reply", "notes.quote", "notes.mention", "notes.renote"]);

/** Applies one realtime frame to the cached notes. Returns nothing; unrelated frames are ignored. */
export function applyNotesFrame(queryClient: QueryClient, frame: ServerFrame): void {
  if (frame.body.case === "notify") {
    const event = frame.body.value.event;
    if (event.case === "received" && NOTE_KINDS.has(event.value.notification?.kind ?? ""))
      hintTimelines();
    return;
  }
  if (frame.body.case !== "notes") return;
  const event = frame.body.value.event;
  switch (event.case) {
    case "countersChanged": {
      const counters = event.value;
      const before = cachedCounters(queryClient, counters.noteId);
      patchNote(queryClient, counters.noteId, (note) => withCounters(note, counters));
      // A new reply, quote or renote of a note on screen is a new row for the timeline.
      if (
        before &&
        (counters.replyCount > before.replyCount ||
          counters.quoteCount > before.quoteCount ||
          counters.renoteCount > before.renoteCount)
      )
        hintTimelines();
      break;
    }
    case "noteDeletedId":
      markNoteDeleted(event.value);
      break;
    case "pollUpdated":
    case "noteEdited":
      // What a reader may see of a poll, or the new words: read the note again.
      void refreshNote(queryClient, event.value.noteId);
      break;
    case "linkPreviewResolved": {
      const { noteId, linkPreview } = event.value;
      patchNote(queryClient, noteId, (note) => ({ ...note, linkPreview }));
      break;
    }
    default:
      break;
  }
}
