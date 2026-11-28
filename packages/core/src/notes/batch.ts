import type { QueryClient } from "@tanstack/query-core";
import type { notes } from "@tokuly/api";
import { type ClientSource, reread } from "../query";
import { notesKeys } from "./keys";
import { putRefreshedNote } from "./live";

/** `BatchGetNotes` takes at most this many IDs. */
const BATCH_LIMIT = 100;

/** Reads one note by ID; `null` when it is gone or hidden from the viewer. */
export type NoteLoader = (noteId: string) => Promise<notes.Note | null>;

interface Waiter {
  noteId: string;
  resolve(note: notes.Note | null): void;
  reject(error: unknown): void;
}

/**
 * Single notes, read in batches: the requests of the same moment (a list of
 * cards appearing, their topics taking effect) share one `BatchGetNotes` call.
 * One per app.
 */
export function noteLoader(client: ClientSource<typeof notes.NoteService>): NoteLoader {
  let waiting: Waiter[] = [];
  let scheduled = false;

  async function flush(): Promise<void> {
    scheduled = false;
    const waiters = waiting;
    waiting = [];
    const ids = [...new Set(waiters.map((waiter) => waiter.noteId))];
    for (let start = 0; start < ids.length; start += BATCH_LIMIT) {
      const chunk = new Set(ids.slice(start, start + BATCH_LIMIT));
      const mine = waiters.filter((waiter) => chunk.has(waiter.noteId));
      try {
        const response = await client().batchGetNotes({ noteIds: [...chunk] });
        const found = new Map(response.notes.map((note) => [note.id, note]));
        for (const waiter of mine) waiter.resolve(found.get(waiter.noteId) ?? null);
      } catch (error) {
        for (const waiter of mine) waiter.reject(error);
      }
    }
  }

  return (noteId) =>
    new Promise((resolve, reject) => {
      waiting.push({ noteId, resolve, reject });
      if (scheduled) return;
      scheduled = true;
      setTimeout(() => void flush(), 10);
    });
}

/**
 * Rule 1 for a note's topic (`docs/realtime-consistency.md`): once the topic
 * is in effect — or was refused, which is what a note the viewer may no
 * longer see gets — the note is read again, since it may have been deleted,
 * edited or voted on between the read that showed it and the subscription.
 * The answer takes the place of every cached copy (`putRefreshedNote`); the
 * note's own query, if any, is read again (`reread`: a read in flight may be
 * older). Resolves to whether the note is gone, which the app then hides.
 */
export async function readNoteAgain(
  queryClient: QueryClient,
  load: NoteLoader,
  noteId: string,
): Promise<boolean> {
  void reread(queryClient, [notesKeys.note(noteId)]);
  const fresh = await load(noteId);
  if (fresh) putRefreshedNote(queryClient, fresh);
  return fresh === null;
}
