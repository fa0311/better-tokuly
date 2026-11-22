import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { common, type MessageInitShape, notes } from "@tokuly/api";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useMe } from "#/stores/session";
import { bumpCounter, findCachedNote, mapNotes, markNoteDeleted, noteClient, notesKeys, patchNote, prependNote, type ThreadData, withCounters, withViewer } from "./api";

const say = (key: string) => i18n.t(key, { ns: "notes" });

/**
 * Likes a note or takes the like back. The heart and the count move at
 * once; the server's numbers replace the guess, a refusal undoes it.
 */
export function useLike(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const liked = !note.viewer?.liked;
    const delta = liked ? 1 : -1;
    patchNote(queryClient, note.id, (current) => (current.viewer?.liked === liked ? current : withViewer({ ...current, counters: bumpCounter(current, "likeCount", delta) }, { liked })));
    try {
      const response = liked ? await noteClient().likeNote({ noteId: note.id }) : await noteClient().unlikeNote({ noteId: note.id });
      if (response.counters) patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
      void queryClient.invalidateQueries({ queryKey: notesKeys.likers(note.id) });
    } catch (error) {
      // Take back only this tap: counts that moved meanwhile (other people, realtime) stay.
      patchNote(queryClient, note.id, (current) => (current.viewer?.liked === liked ? withViewer({ ...current, counters: bumpCounter(current, "likeCount", -delta) }, { liked: !liked }) : current));
      toast.error(errorMessage(error));
    }
  };
}

/** Renotes a public note to the viewer's friends, or takes the renote back. */
export function useRenote(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const renoted = !note.viewer?.renoted;
    const delta = renoted ? 1 : -1;
    patchNote(queryClient, note.id, (current) => (current.viewer?.renoted === renoted ? current : withViewer({ ...current, counters: bumpCounter(current, "renoteCount", delta) }, { renoted })));
    try {
      const response = renoted ? await noteClient().renoteNote({ noteId: note.id }) : await noteClient().undoRenote({ noteId: note.id });
      if (response.counters) patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
      void queryClient.invalidateQueries({ queryKey: notesKeys.renoters(note.id) });
      toast.success(say(renoted ? "toast.renoted" : "toast.renoteUndone"));
    } catch (error) {
      patchNote(queryClient, note.id, (current) => (current.viewer?.renoted === renoted ? withViewer({ ...current, counters: bumpCounter(current, "renoteCount", -delta) }, { renoted: !renoted }) : current));
      toast.error(errorMessage(error));
    }
  };
}

type Counter = "replyCount" | "quoteCount";

/**
 * Moves a counter of the note a reply or a quote hangs off, once the server
 * has confirmed the change — unless the count already moved meanwhile: the
 * realtime push with the new total often arrives before the answer does,
 * and the change must not be counted twice.
 */
function settleCounter(queryClient: QueryClient, noteId: string | undefined, counter: Counter, before: bigint | undefined, delta: number): void {
  if (!noteId) return;
  patchNote(queryClient, noteId, (note) => ((note.counters?.[counter] ?? 0n) === (before ?? 0n) ? { ...note, counters: bumpCounter(note, counter, delta) } : note));
}

const counterNow = (queryClient: QueryClient, noteId: string | undefined, counter: Counter): bigint | undefined => (noteId ? findCachedNote(queryClient, noteId)?.counters?.[counter] : undefined);

/** Deletes one of the viewer's notes. It disappears from every list on screen. */
export function useDeleteNote(): (note: notes.Note) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note) => {
    const repliesBefore = counterNow(queryClient, note.replyTo?.noteId, "replyCount");
    const quotesBefore = counterNow(queryClient, note.quote?.noteId, "quoteCount");
    try {
      await noteClient().deleteNote({ noteId: note.id });
    } catch (error) {
      toast.error(errorMessage(error));
      return false;
    }
    markNoteDeleted(note.id);
    // The note it answered or quoted lost a reply or a quote.
    settleCounter(queryClient, note.replyTo?.noteId, "replyCount", repliesBefore, -1);
    settleCounter(queryClient, note.quote?.noteId, "quoteCount", quotesBefore, -1);
    toast(say("toast.deleted"));
    return true;
  };
}

/** Pins a note to the viewer's profile (replacing the one pinned before) or removes the pin. */
export function usePin(): (note: notes.Note, pinned: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  const me = useMe();
  return async (note, pinned) => {
    try {
      if (pinned) await noteClient().pinNote({ noteId: note.id });
      else await noteClient().unpinNote({});
    } catch (error) {
      toast.error(errorMessage(error));
      return;
    }
    // One pin per profile: every other note of the viewer loses the flag.
    for (const query of queryClient.getQueryCache().findAll({ queryKey: notesKeys.all })) {
      const data = query.state.data;
      if (data === undefined) continue;
      const next = mapNotes(data, (current) => {
        const shouldBePinned = pinned && current.id === note.id;
        return current.author?.id === note.author?.id && current.pinned !== shouldBePinned ? { ...current, pinned: shouldBePinned } : current;
      });
      if (next !== data) queryClient.setQueryData(query.queryKey, next, { updatedAt: query.state.dataUpdatedAt });
    }
    if (me?.user) void queryClient.invalidateQueries({ queryKey: notesKeys.userTimelines(me.user.id) });
    toast.success(say(pinned ? "toast.pinned" : "toast.unpinned"));
  };
}

/** Changes who may see one of the viewer's notes. The visibility the server applied is what the card then shows. */
export function useSetVisibility(): (note: notes.Note, visibility: common.Visibility) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note, visibility) => {
    try {
      const response = await noteClient().setNoteVisibility({ noteId: note.id, visibility });
      const applied = response.note?.visibility ?? visibility;
      patchNote(queryClient, note.id, (current) => ({ ...current, visibility: applied }));
      toast.success(say("toast.visibilityChanged"));
      return true;
    } catch (error) {
      toast.error(errorMessage(error));
      return false;
    }
  };
}

/**
 * A note the viewer just posted takes its place in the lists already on
 * screen. `before` holds the reply and quote counts of the note it answers
 * or quotes as they were when the request was sent.
 */
function addPostedNote(queryClient: QueryClient, note: notes.Note, before: { replies?: bigint; quotes?: bigint }): void {
  const authorId = note.author?.id;
  const isReply = Boolean(note.replyTo?.noteId);
  prependNote(queryClient, { kind: "home" }, note);
  if (!isReply && note.visibility === common.Visibility.PUBLIC) prependNote(queryClient, { kind: "global" }, note);
  for (const tag of note.hashtags) prependNote(queryClient, { kind: "hashtag", tag }, note);
  if (authorId) {
    if (!isReply) prependNote(queryClient, { kind: "user", userId: authorId, filter: notes.UserTimelineFilter.NOTES }, note);
    prependNote(queryClient, { kind: "user", userId: authorId, filter: notes.UserTimelineFilter.WITH_REPLIES }, note);
    if (note.attachments.length > 0) void queryClient.invalidateQueries({ queryKey: notesKeys.timeline({ kind: "user", userId: authorId, filter: notes.UserTimelineFilter.MEDIA }) });
  }

  const parentId = note.replyTo?.noteId;
  if (parentId) {
    settleCounter(queryClient, parentId, "replyCount", before.replies, 1);
    // Replies are listed oldest first: the new one belongs at the end, which is only on screen once every page is.
    queryClient.setQueryData<ThreadData>(notesKeys.thread(parentId), (thread) => {
      const last = thread?.pages.at(-1);
      if (!thread || !last || last.page?.nextCursor) return thread;
      return { ...thread, pages: thread.pages.map((page) => (page === last ? { ...page, replies: [...page.replies.filter((reply) => reply.id !== note.id), note] } : page)) };
    });
  }
  const quotedId = note.quote?.noteId;
  if (quotedId) {
    settleCounter(queryClient, quotedId, "quoteCount", before.quotes, 1);
    void queryClient.invalidateQueries({ queryKey: notesKeys.quotes(quotedId) });
  }
}

export type NewNote = MessageInitShape<typeof notes.CreateNoteRequestSchema>;

/** Posts a note. `idempotencyKey` makes a retry after a lost answer safe. */
export async function postNote(queryClient: QueryClient, request: NewNote): Promise<notes.Note> {
  const before = { replies: counterNow(queryClient, request.replyToId, "replyCount"), quotes: counterNow(queryClient, request.quoteOfId, "quoteCount") };
  const response = await noteClient().createNote(request);
  const note = response.note!;
  addPostedNote(queryClient, note, before);
  return note;
}
