import type { QueryClient } from "@tanstack/query-core";
import type { notes } from "@tokuly/api";
import { patchNote, withCounters } from "./cache";
import { notesKeys } from "./keys";
import type { TimelineData } from "./timeline";

/**
 * What an app still has to do after a notes event went into the cache:
 * `hint` — something new may belong on top of a timeline (look);
 * `deleted` — the note is gone (hide it wherever it shows);
 * `refresh` — read the note again (its words or its poll changed).
 */
export interface NotesEventEffect {
  hint?: boolean;
  deleted?: string;
  refresh?: string;
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

/**
 * Applies one realtime notes event to the cached notes (counters, link
 * previews) and says what is left for the app. The connection has no event
 * for a friend's new note, so a reply, quote or renote count going up on a
 * note on screen is the hint that a timeline has something new.
 */
export function applyNotesEvent(
  queryClient: QueryClient,
  event: notes.NotesEvent["event"],
): NotesEventEffect {
  switch (event.case) {
    case "countersChanged": {
      const counters = event.value;
      const before = cachedCounters(queryClient, counters.noteId);
      patchNote(queryClient, counters.noteId, (note) => withCounters(note, counters));
      const grew =
        before !== undefined &&
        (counters.replyCount > before.replyCount ||
          counters.quoteCount > before.quoteCount ||
          counters.renoteCount > before.renoteCount);
      return grew ? { hint: true } : {};
    }
    case "noteDeletedId":
      return { deleted: event.value };
    case "pollUpdated":
    case "noteEdited":
      return { refresh: event.value.noteId };
    case "linkPreviewResolved": {
      const { noteId, linkPreview } = event.value;
      patchNote(queryClient, noteId, (note) => ({ ...note, linkPreview }));
      return {};
    }
    default:
      return {};
  }
}

/** Notification kinds that mean a note now belongs in the home or mentions timeline. */
const NOTE_KINDS = new Set(["notes.reply", "notes.quote", "notes.mention", "notes.renote"]);

/** Whether a notification of this kind is a hint for the timelines. */
export const isNoteNotification = (kind: string): boolean => NOTE_KINDS.has(kind);
