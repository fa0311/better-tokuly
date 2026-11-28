import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { common, identity, notes, social } from "@tokuly/api";
import { isCloseFriends } from "@tokuly/core/friends";
import {
  clipKeys,
  listKeys,
  loadTimelinePage,
  noteLoader,
  notesKeys,
  PAGE_SIZE,
  type TimelineClients,
  type TimelineSource,
  timelineQuery,
} from "@tokuly/core/notes";
import { nextCursor } from "@tokuly/core/query";
import { api } from "#/lib/api";

/**
 * Notes in the query cache. Timelines, the cache edits and the rules of a
 * note are `@tokuly/core/notes`; this file wires them to the phone's API
 * client. Every query lives in the signed-in tree's cache, so another
 * account never sees these.
 */
export const noteClient = () => api().service(notes.NoteService);
export const draftClient = () => api().service(notes.NoteDraftService);
export const listClient = () => api().service(notes.NoteListService);
export const antennaClient = () => api().service(notes.AntennaService);
export const clipClient = () => api().service(notes.ClipService);
export const muteClient = () => api().service(notes.NoteMuteService);

/** One note by ID; `null` when it is gone or hidden. Requests of the same moment share one `BatchGetNotes` call. */
export const loadNote = noteLoader(noteClient);

const timelineClients: TimelineClients = {
  notes: noteClient,
  lists: listClient,
  antennas: antennaClient,
};

/** The newest page of a timeline, read now: what goes on top of the one on screen. */
export const freshPage = (source: TimelineSource) => loadTimelinePage(timelineClients, source, "");

export function useTimeline(source: TimelineSource) {
  return useInfiniteQuery(timelineQuery(timelineClients, source));
}

/** A note in its conversation: the notes it answers, itself, and its replies oldest first (paged). */
export function useThread(noteId: string) {
  return useInfiniteQuery({
    queryKey: notesKeys.thread(noteId),
    queryFn: ({ pageParam }) =>
      noteClient().getNoteThread({ noteId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

/** One note; `null` when it is gone or hidden. Kept current by the same live updates as timelines. */
export function useNote(noteId: string | undefined) {
  return useQuery({
    queryKey: notesKeys.note(noteId ?? ""),
    queryFn: async () => (await noteClient().getNote({ noteId: noteId ?? "" })).note ?? null,
    enabled: Boolean(noteId),
    staleTime: 60_000,
  });
}

/** The posting rules and whether the viewer agreed to the version in force. */
export function useNoteRules() {
  return useQuery({
    queryKey: notesKeys.rules,
    queryFn: () => noteClient().getNoteRules({}),
    staleTime: 5 * 60_000,
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

/** The viewer's drafts (scheduled ones too), most recently changed first. */
export function useDrafts() {
  return useInfiniteQuery({
    queryKey: notesKeys.drafts,
    queryFn: ({ pageParam }) =>
      draftClient().listDrafts({ page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

export function useDraft(draftId: string | undefined) {
  return useQuery({
    queryKey: [...notesKeys.drafts, "one", draftId ?? ""] as const,
    queryFn: async () => (await draftClient().getDraft({ draftId: draftId ?? "" })).draft ?? null,
    enabled: Boolean(draftId),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** The viewer's own lists and the ones they follow: timelines to read. */
export function useReadableLists() {
  const mine = useQuery({
    queryKey: listKeys.mine,
    queryFn: async () => (await listClient().listLists({})).lists,
    staleTime: 60_000,
  });
  const followed = useQuery({
    queryKey: listKeys.followed,
    queryFn: async () => (await listClient().listFollowedLists({ page: { limit: 50 } })).lists,
    staleTime: 60_000,
  });
  return { mine, followed };
}

export function useList(listId: string) {
  return useQuery({
    queryKey: listKeys.list(listId),
    queryFn: async () => (await listClient().getList({ listId })).list ?? null,
  });
}

export function useAntennas() {
  return useQuery({
    queryKey: listKeys.antennas,
    queryFn: async () => (await antennaClient().listAntennas({})).antennas,
    staleTime: 60_000,
  });
}

export function useAntenna(antennaId: string) {
  return useQuery({
    queryKey: listKeys.antenna(antennaId),
    queryFn: async () => (await antennaClient().getAntenna({ antennaId })).antenna ?? null,
  });
}

/** The viewer's clips; with `noteId`, each says whether it holds that note. */
export function useClips(noteId = "", enabled = true) {
  return useInfiniteQuery({
    queryKey: clipKeys.list("", noteId),
    queryFn: ({ pageParam }) =>
      clipClient().listClips({ noteId, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled,
  });
}

export function useClip(clipId: string) {
  return useQuery({
    queryKey: clipKeys.clip(clipId),
    queryFn: async () => (await clipClient().getClip({ clipId })).clip ?? null,
  });
}

/** The notes of a clip, most recently added first. */
export function useClipNotes(clipId: string) {
  return useInfiniteQuery({
    queryKey: clipKeys.notes(clipId),
    queryFn: ({ pageParam }) =>
      clipClient().getClipNotes({ clipId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

/**
 * Who a note goes to when nothing is picked: the account's own setting (a
 * minor's narrower default is already in it), so the composer can say it.
 * The note is still sent without an audience: the server applies the setting.
 */
export function useDefaultAudience() {
  return useQuery({
    queryKey: [...notesKeys.all, "default-audience"] as const,
    queryFn: async () => {
      const { settings } = await api().service(identity.AccountService).getSettings({});
      return { visibility: settings?.privacy?.defaultNoteVisibility || common.Visibility.PUBLIC };
    },
    staleTime: 60_000,
  });
}

/** The viewer's custom circles (close friends has its own choice): audiences of a note. */
export function useCustomCircles(enabled: boolean) {
  return useQuery({
    queryKey: notesKeys.circles,
    queryFn: async () =>
      (await api().service(social.CircleService).listCircles({})).circles.filter(
        (circle) => !isCloseFriends(circle),
      ),
    enabled,
    staleTime: 60_000,
  });
}

/** The viewer's friends, to address a direct note to. */
export function useDirectCandidates(enabled: boolean) {
  return useQuery({
    queryKey: notesKeys.directCandidates,
    queryFn: async () =>
      (
        await api()
          .service(social.FriendService)
          .listFriends({ page: { limit: 100 } })
      ).friends.flatMap((friend) => (friend.user ? [friend.user] : [])),
    enabled,
    staleTime: 60_000,
  });
}

// ───────────────────────────── Deleted notes ─────────────────────────────

/**
 * Notes known to be gone (deleted here, or announced as deleted over
 * realtime): lists hide them without anyone rewriting cached pages. Kept in
 * the query cache, so they belong to the account that learnt of them.
 */
const DELETED = ["notes-deleted"] as const;

export function markNoteDeleted(queryClient: QueryClient, noteId: string): void {
  queryClient.setQueryData<readonly string[]>(DELETED, (ids = []) =>
    ids.includes(noteId) ? ids : [...ids, noteId],
  );
}

/** The IDs of the notes known to be gone. */
export function useDeletedNotes(): ReadonlySet<string> {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: DELETED,
    queryFn: () => queryClient.getQueryData<readonly string[]>(DELETED) ?? [],
    initialData: [] as readonly string[],
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    select: (ids) => new Set(ids),
  });
  return data;
}
