import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { common, errorReason, type MessageInitShape, type notes } from "@tokuly/api";
import {
  type Engagement,
  emojiReaction,
  engagementKeys,
  reactionFields,
  type Subject,
  withReactionChange,
} from "@tokuly/core/engagement";
import {
  addPostedNote,
  clipKeys,
  countsBefore,
  notesKeys,
  patchNote,
  settleDeletedNote,
  withCounters,
  withToggle,
  withViewer,
} from "@tokuly/core/notes";
import { Alert } from "react-native";
import { reactionClient } from "#/features/engagement/data";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { clipClient, draftClient, markNoteDeleted, muteClient, noteClient } from "./data";

/**
 * What the viewer does with a note. Each action takes its client before
 * the first await (the account that tapped is the one that acts, even if
 * another takes over meanwhile), shows the change at once where that is
 * safe, and takes it back when the server says no.
 */
const fail = (error: unknown) => Alert.alert(errorMessage(error, { namespace: "notes" }));

export const noteSubject = (noteId: string): Subject => ({
  type: common.SubjectType.NOTE,
  id: noteId,
});

/** Likes a note or takes the like back: the heart and the count move at once. */
export function useLike(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const client = noteClient();
    const liked = !note.viewer?.liked;
    patchNote(queryClient, note.id, (current) => withToggle(current, "like", liked));
    try {
      const response = liked
        ? await client.likeNote({ noteId: note.id })
        : await client.unlikeNote({ noteId: note.id });
      if (response.counters)
        patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
    } catch (error) {
      // Take back only this tap: counts that moved meanwhile (other people, realtime) stay.
      patchNote(queryClient, note.id, (current) =>
        current.viewer?.liked === liked ? withToggle(current, "like", !liked) : current,
      );
      fail(error);
    }
  };
}

/** Renotes a note to the viewer's friends, or takes the renote back. */
export function useRenote(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const client = noteClient();
    const renoted = !note.viewer?.renoted;
    patchNote(queryClient, note.id, (current) => withToggle(current, "renote", renoted));
    try {
      const response = renoted
        ? await client.renoteNote({ noteId: note.id })
        : await client.undoRenote({ noteId: note.id });
      if (response.counters)
        patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
    } catch (error) {
      patchNote(queryClient, note.id, (current) =>
        current.viewer?.renoted === renoted ? withToggle(current, "renote", !renoted) : current,
      );
      fail(error);
    }
  };
}

/** Deletes one of the viewer's notes; it leaves every list on screen. */
export function useDeleteNote(): (note: notes.Note) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note) => {
    const client = noteClient();
    const before = countsBefore(queryClient, {
      replyToId: note.replyTo?.noteId,
      quoteOfId: note.quote?.noteId,
    });
    try {
      await client.deleteNote({ noteId: note.id });
    } catch (error) {
      fail(error);
      return false;
    }
    markNoteDeleted(queryClient, note.id);
    settleDeletedNote(queryClient, note, before);
    return true;
  };
}

/** Pins a note to the viewer's profile (up to five) or takes it off. */
export function usePin(): (note: notes.Note, pinned: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note, pinned) => {
    const client = noteClient();
    try {
      if (pinned) await client.pinNote({ noteId: note.id });
      else await client.unpinNote({ noteId: note.id });
    } catch (error) {
      return fail(error);
    }
    patchNote(queryClient, note.id, (current) => ({ ...current, pinned }));
    if (note.author)
      void queryClient.invalidateQueries({ queryKey: notesKeys.userTimelines(note.author.id) });
  };
}

/** Stops (or restarts) the notifications of the conversation a note is in. */
export function useMuteThread(): (note: notes.Note, muted: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note, muted) => {
    const client = muteClient();
    try {
      if (muted) await client.muteThread({ noteId: note.id });
      else await client.unmuteThread({ noteId: note.id });
    } catch (error) {
      return fail(error);
    }
    patchNote(queryClient, note.id, (current) => withViewer(current, { threadMuted: muted }));
  };
}

/** Keeps someone's renotes out of the viewer's timelines (their own notes still show). */
export function useHideRenotes(): (userId: string) => Promise<void> {
  return async (userId) => {
    const client = muteClient();
    try {
      await client.hideRenotesFrom({ userId });
      Alert.alert(i18n.t("notes:mutes.renotesHidden"));
    } catch (error) {
      fail(error);
    }
  };
}

/** Votes in a note's poll (votes are final and private); the poll as the server then shows it replaces the card's. */
export function useVote(): (note: notes.Note, choices: number[]) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note, choices) => {
    const client = noteClient();
    try {
      const { poll } = await client.votePoll({ noteId: note.id, choices });
      if (poll) patchNote(queryClient, note.id, (current) => ({ ...current, poll }));
      return true;
    } catch (error) {
      fail(error);
      return false;
    }
  };
}

/**
 * An emoji reaction to a note (the engagement rules of the subject: a new
 * one replaces the old unless the note takes several), or taking it back.
 */
export function useReact(noteId: string): (emoji: string, on: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  return async (emoji, on) => {
    const subject = noteSubject(noteId);
    const key = engagementKeys.subject(subject);
    const reactions = reactionClient();
    const reaction = emojiReaction(emoji);
    await queryClient.cancelQueries({ queryKey: key, exact: true });
    const before = queryClient.getQueryData<Engagement | null>(key);
    if (before)
      queryClient.setQueryData(
        key,
        withReactionChange(before, on ? { add: reaction } : { remove: reaction }),
      );
    try {
      const response = on
        ? await reactions.setReaction({ subject, reaction: { kind: reaction.kind, emoji } })
        : await reactions.removeReaction({ subject, reaction: { kind: reaction.kind, emoji } });
      queryClient.setQueryData<Engagement | null>(key, (current) =>
        current ? { ...current, ...reactionFields(response.reactions) } : current,
      );
    } catch (error) {
      queryClient.setQueryData(key, before);
      fail(error);
    }
  };
}

/** Puts a note into one of the viewer's clips or takes it out; the sheet's ticks follow the server. */
export function useClipNote(): (clip: notes.Clip, noteId: string) => Promise<void> {
  const queryClient = useQueryClient();
  return async (clip, noteId) => {
    const client = clipClient();
    try {
      if (clip.containsNote) await client.removeNoteFromClip({ clipId: clip.id, noteId });
      else await client.addNoteToClip({ clipId: clip.id, noteId });
    } catch (error) {
      return fail(error);
    }
    void queryClient.invalidateQueries({ queryKey: clipKeys.all });
  };
}

/** Makes a clip and puts the note into it. */
export function useNewClip(): (name: string, noteId: string) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (name, noteId) => {
    const client = clipClient();
    try {
      const { clip } = await client.createClip({ name, description: "", public: false });
      if (clip) await client.addNoteToClip({ clipId: clip.id, noteId });
    } catch (error) {
      fail(error);
      return false;
    }
    void queryClient.invalidateQueries({ queryKey: clipKeys.all });
    return true;
  };
}

/** Agrees to the posting rules in the version shown. */
export async function agreeToRules(queryClient: QueryClient, version: number): Promise<void> {
  const client = noteClient();
  await client.agreeToNoteRules({ version });
  await queryClient.invalidateQueries({ queryKey: notesKeys.rules });
}

export type NewNote = MessageInitShape<typeof notes.CreateNoteRequestSchema>;
/** What is written, as CreateNote takes it and a draft keeps it (everything but the idempotency key). */
export type NoteContent = Omit<Exclude<NewNote, notes.CreateNoteRequest>, "idempotencyKey">;

/**
 * Posts a note with `client` (the account's, taken when the person tapped
 * "post"). `idempotencyKey` makes a retry after a lost answer safe. A
 * refusal because the rules changed brings the agreement up again.
 */
export async function postNote(
  queryClient: QueryClient,
  client: ReturnType<typeof noteClient>,
  request: NewNote,
): Promise<notes.Note> {
  const before = countsBefore(queryClient, request);
  try {
    const { note } = await client.createNote(request);
    addPostedNote(queryClient, note!, before);
    return note!;
  } catch (error) {
    if (errorReason(error) === "RULES_NOT_ACCEPTED")
      void queryClient.invalidateQueries({ queryKey: notesKeys.rules });
    throw error;
  }
}

/** Keeps what is being written as a server draft (a new one, or `draftId` replaced). */
export async function saveDraft(
  queryClient: QueryClient,
  client: ReturnType<typeof draftClient>,
  draftId: string,
  content: NoteContent,
): Promise<void> {
  await client.saveDraft({ draftId, content });
  void queryClient.invalidateQueries({ queryKey: notesKeys.drafts });
}

/** A draft goes out now (as a note) and leaves the drafts. */
export function usePublishDraft(): (draft: notes.Draft) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (draft) => {
    const client = draftClient();
    const before = countsBefore(queryClient, {
      replyToId: draft.content?.replyToId,
      quoteOfId: draft.content?.quoteOfId,
    });
    try {
      const { note } = await client.publishDraft({ draftId: draft.id });
      if (note) addPostedNote(queryClient, note, before);
    } catch (error) {
      fail(error);
      return false;
    } finally {
      void queryClient.invalidateQueries({ queryKey: notesKeys.drafts });
    }
    Alert.alert(i18n.t("notes:toast.posted"));
    return true;
  };
}

export function useDeleteDraft(): (draft: notes.Draft) => Promise<void> {
  const queryClient = useQueryClient();
  return async (draft) => {
    const client = draftClient();
    try {
      await client.deleteDraft({ draftId: draft.id });
    } catch (error) {
      return fail(error);
    }
    void queryClient.invalidateQueries({ queryKey: notesKeys.drafts });
  };
}
