import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { forgetUserNotes } from "@tokuly/core/notes";
import { useEffect } from "react";
import { registerSubjectCard } from "#/features/engagement";
import { realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { resetDeletedNotes } from "./api";
import { ComposeHost } from "./compose";
import { closeCompose } from "./compose-store";
import { discardAllDrafts } from "./drafts";
import { applyNotesFrame, hintTimelines } from "./live";
import { NoteSubjectCard } from "./subject-card";
import { resetViews } from "./views";

// Saved and reacted-to notes are drawn as notes on `/bookmarks`.
registerSubjectCard(common.SubjectType.NOTE, NoteSubjectCard);

/**
 * Runs while someone is signed in: keeps cached notes current from the
 * realtime connection (counters, deletions, link previews), tells timelines
 * when to look for new notes, drops the notes of someone who stopped being a
 * friend, and hosts the composer so any page can open it. When the session
 * ends, what this browser kept for the account (drafts, reported views, the
 * notes known to be deleted) goes with it.
 */
export default function NotesBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Whatever was posted while the connection was down was not announced.
        if (connectedBefore) hintTimelines();
        connectedBefore = true;
        return;
      }
      // A friendship ended, or the other side put up a block.
      if (frame.body.case === "social" && frame.body.value.event.case === "friendRemovedUserId")
        forgetUserNotes(queryClient, frame.body.value.event.value);
      applyNotesFrame(queryClient, frame);
    });
    client.start();
    const onVisible = () => {
      if (document.visibilityState === "visible") hintTimelines();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      off();
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [queryClient]);

  useEffect(
    () =>
      useSessionStore.subscribe((session) => {
        if (session.state.status !== "signedOut") return;
        closeCompose();
        discardAllDrafts();
        resetViews();
        resetDeletedNotes();
      }),
    [],
  );

  return <ComposeHost />;
}
