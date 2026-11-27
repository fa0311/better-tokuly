import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import type { ServerFrame } from "@tokuly/realtime";
import { lazy, useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { ComposeHost } from "./compose-host";
import { hintTimelines } from "./timeline-hints";

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
    const offWelcome = client.onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      // Whatever was posted while the connection was down was not announced.
      if (connectedBefore) hintTimelines();
      connectedBefore = true;
    });
    const applyFrame = () =>
      import("./live").then(
        ({ applyNotesFrame }) =>
          (_: unknown, frame: ServerFrame) =>
            applyNotesFrame(queryClient, frame),
      );
    const offNotes = onFramesLoading("notes", applyFrame);
    const offNotify = onFramesLoading("notify", applyFrame);
    // A friendship ended, or the other side put up a block.
    const offSocial = onFramesLoading("social", () =>
      import("./live").then(
        ({ applySocialEvent }) =>
          ({ event }) =>
            applySocialEvent(queryClient, event),
      ),
    );
    client.start();
    const onVisible = () => {
      if (document.visibilityState === "visible") hintTimelines();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      offWelcome();
      offNotes();
      offNotify();
      offSocial();
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [queryClient]);

  return <ComposeHost />;
}
