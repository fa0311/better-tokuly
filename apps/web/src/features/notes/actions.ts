import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import type { MessageInitShape, notes } from "@tokuly/api";
import {
  type Audience,
  addPostedNote,
  countsBefore,
  notesKeys,
  patchNote,
  settleDeletedNote,
  withCounters,
  withToggle,
} from "@tokuly/core/notes";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useMe } from "#/stores/session";
import { markNoteDeleted, noteClient } from "./api";

const say = (key: string) => i18n.t(key, { ns: "notes" });

/**
 * Likes a note or takes the like back. The heart and the count move at
 * once; the server's numbers replace the guess, a refusal undoes it.
 */
export function useLike(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const liked = !note.viewer?.liked;
    patchNote(queryClient, note.id, (current) => withToggle(current, "like", liked));
    try {
      const response = liked
        ? await noteClient().likeNote({ noteId: note.id })
        : await noteClient().unlikeNote({ noteId: note.id });
      if (response.counters)
        patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
      void queryClient.invalidateQueries({ queryKey: notesKeys.likers(note.id) });
    } catch (error) {
      // Take back only this tap: counts that moved meanwhile (other people, realtime) stay.
      patchNote(queryClient, note.id, (current) =>
        current.viewer?.liked === liked ? withToggle(current, "like", !liked) : current,
      );
      toast.error(errorMessage(error, { namespace: "notes" }));
    }
  };
}

/** Renotes a public note to the viewer's friends, or takes the renote back. */
export function useRenote(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  return async (note) => {
    const renoted = !note.viewer?.renoted;
    patchNote(queryClient, note.id, (current) => withToggle(current, "renote", renoted));
    try {
      const response = renoted
        ? await noteClient().renoteNote({ noteId: note.id })
        : await noteClient().undoRenote({ noteId: note.id });
      if (response.counters)
        patchNote(queryClient, note.id, (current) => withCounters(current, response.counters!));
      void queryClient.invalidateQueries({ queryKey: notesKeys.renoters(note.id) });
      toast.success(say(renoted ? "toast.renoted" : "toast.renoteUndone"));
    } catch (error) {
      patchNote(queryClient, note.id, (current) =>
        current.viewer?.renoted === renoted ? withToggle(current, "renote", !renoted) : current,
      );
      toast.error(errorMessage(error, { namespace: "notes" }));
    }
  };
}

/** Deletes one of the viewer's notes. It disappears from every list on screen. */
export function useDeleteNote(): (note: notes.Note) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note) => {
    const before = countsBefore(queryClient, {
      replyToId: note.replyTo?.noteId,
      quoteOfId: note.quote?.noteId,
    });
    try {
      await noteClient().deleteNote({ noteId: note.id });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
      return false;
    }
    markNoteDeleted(note.id);
    settleDeletedNote(queryClient, note, before);
    toast(say("toast.deleted"));
    return true;
  };
}

/** Pins a note to the viewer's profile (up to five) or takes it off. */
export function usePin(): (note: notes.Note, pinned: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  const me = useMe();
  return async (note, pinned) => {
    try {
      if (pinned) await noteClient().pinNote({ noteId: note.id });
      else await noteClient().unpinNote({ noteId: note.id });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
      return;
    }
    patchNote(queryClient, note.id, (current) => ({ ...current, pinned }));
    if (me?.user)
      void queryClient.invalidateQueries({ queryKey: notesKeys.userTimelines(me.user.id) });
    toast.success(say(pinned ? "toast.pinned" : "toast.unpinned"));
  };
}

/** Changes who may see one of the viewer's notes. The audience the server applied is what the card then shows. */
export function useSetVisibility(): (note: notes.Note, audience: Audience) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (note, audience) => {
    try {
      const response = await noteClient().setNoteVisibility({
        noteId: note.id,
        visibility: audience.visibility,
        circleId: audience.circleId ?? "",
        directUserIds: audience.directUserIds ?? [],
      });
      const applied = response.note;
      patchNote(queryClient, note.id, (current) =>
        applied
          ? {
              ...current,
              visibility: applied.visibility,
              circleId: applied.circleId,
              directRecipients: applied.directRecipients,
              viewer: applied.viewer,
            }
          : current,
      );
      toast.success(say("toast.visibilityChanged"));
      return true;
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
      return false;
    }
  };
}

export type NewNote = MessageInitShape<typeof notes.CreateNoteRequestSchema>;

/** Posts a note. `idempotencyKey` makes a retry after a lost answer safe. */
export async function postNote(queryClient: QueryClient, request: NewNote): Promise<notes.Note> {
  const before = countsBefore(queryClient, request);
  const response = await noteClient().createNote(request);
  const note = response.note!;
  addPostedNote(queryClient, note, before);
  return note;
}
