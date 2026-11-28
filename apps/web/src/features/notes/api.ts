import { type QueryClient, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import {
  loadTimelinePage,
  noteLoader,
  notesKeys,
  PAGE_SIZE,
  readNoteAgain,
  type TimelineClients,
  type TimelineSource,
  timelineQuery,
} from "@tokuly/core/notes";
import { create as createStore } from "zustand";
import { api } from "#/lib/api";

export const noteClient = () => api().service(notes.NoteService);
export const draftClient = () => api().service(notes.NoteDraftService);
export const muteClient = () => api().service(notes.NoteMuteService);

const timelineClients: TimelineClients = {
  notes: noteClient,
  lists: () => api().service(notes.NoteListService),
  antennas: () => api().service(notes.AntennaService),
};

/** Reads the newest page of a timeline: what goes on top of the one on screen. */
export const freshPage = (source: TimelineSource) => loadTimelinePage(timelineClients, source, "");

/** A timeline, newest first, paged (`timelineQuery`). */
export function useTimeline(source: TimelineSource, enabled = true) {
  return useInfiniteQuery({ ...timelineQuery(timelineClients, source), enabled });
}

/** A note in its conversation: the notes it answers, itself, and its replies oldest first (paged). */
export function useThread(noteId: string) {
  return useInfiniteQuery({
    queryKey: notesKeys.thread(noteId),
    queryFn: ({ pageParam }) =>
      noteClient().getNoteThread({ noteId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** Quote notes of a note, newest first. */
export function useQuotes(noteId: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: notesKeys.quotes(noteId),
    queryFn: ({ pageParam }) =>
      noteClient().listNoteQuotes({ noteId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

// ───────────────────────────── Single notes, batched ─────────────────────────────

/** One note by ID; `null` when it is gone or hidden. Requests of the same moment share one `BatchGetNotes` call. */
export const loadNote = noteLoader(noteClient);

/** A note shown outside a timeline (a saved note, a feed card). Kept current by the same live updates as timelines. */
export function useNote(noteId: string, enabled = true) {
  return useQuery({
    queryKey: notesKeys.note(noteId),
    queryFn: () => loadNote(noteId),
    enabled: enabled && Boolean(noteId),
    staleTime: 60_000,
  });
}

// ───────────────────────────── Deleted notes ─────────────────────────────

/**
 * Notes known to be gone (deleted here, or announced as deleted over
 * realtime). Lists hide them and quote embeds show them as unavailable
 * without anyone rewriting cached pages.
 */
export const useDeletedNotes = createStore<{ ids: ReadonlySet<string> }>(() => ({
  ids: new Set(),
}));

export function markNoteDeleted(noteId: string): void {
  const { ids } = useDeletedNotes.getState();
  if (!ids.has(noteId)) useDeletedNotes.setState({ ids: new Set(ids).add(noteId) });
}

export function useIsDeleted(noteId: string | undefined): boolean {
  return useDeletedNotes((state) => (noteId ? state.ids.has(noteId) : false));
}

/** A card's topic took effect, or was refused: its note is read again, and hidden when it is gone. */
export function readCardAgain(queryClient: QueryClient, noteId: string): void {
  readNoteAgain(queryClient, loadNote, noteId).then(
    (gone) => gone && markNoteDeleted(noteId),
    // Not read: the next push, or the next time the topic takes effect, tries again.
    () => {},
  );
}

/** Nothing known to be deleted anymore: the next account starts clean. */
export function resetDeletedNotes(): void {
  useDeletedNotes.setState({ ids: new Set() });
}

// ───────────────────────────── Hashtags ─────────────────────────────

/** Hashtags many different people are using right now, best first. */
export function useTrendingHashtags(limit = 10) {
  return useQuery({
    queryKey: [...notesKeys.trending, limit] as const,
    queryFn: () => noteClient().listTrendingHashtags({ limit }),
    // The server recomputes the ranking every few minutes.
    staleTime: 3 * 60_000,
  });
}

/** Hashtags starting with `prefix`, most used first: completion while writing. */
export function useHashtagSearch(prefix: string, enabled: boolean) {
  return useQuery({
    queryKey: notesKeys.hashtagSearch(prefix),
    queryFn: async () => (await noteClient().searchHashtags({ prefix, limit: 8 })).hashtags,
    enabled,
    staleTime: 60_000,
  });
}
