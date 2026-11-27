import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { forgetUserNotes } from "@tokuly/core/notes";
import { lazy, useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { ComposeHost } from "./compose-host";
import { applyNotesFrame, hintTimelines } from "./live";

const NoteSubjectCard = lazy(() =>
  import("./subject-card").then((module) => ({ default: module.NoteSubjectCard })),
);

// Saved and reacted-to notes are drawn as notes on `/bookmarks`. Registered once engagement has
// loaded, off the way of the first page: a list drawn before then draws again.
void import("#/features/engagement").then(({ registerSubjectCard }) =>
  registerSubjectCard(common.SubjectType.NOTE, NoteSubjectCard),
);

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

  return <ComposeHost />;
}
