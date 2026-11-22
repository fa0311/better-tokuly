import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { useEffect } from "react";
import { registerSubjectCard } from "#/features/engagement";
import { realtime } from "#/lib/realtime";
import { ComposeHost } from "./compose";
import { applyNotesFrame, hintTimelines } from "./live";
import { NoteSubjectCard } from "./subject-card";

// Saved and reacted-to notes are drawn as notes on `/bookmarks`.
registerSubjectCard(common.SubjectType.NOTE, NoteSubjectCard);

/**
 * Runs while someone is signed in: keeps cached notes current from the
 * realtime connection (counters, deletions, link previews), tells timelines
 * when to look for new notes, and hosts the composer so any page can open it.
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
