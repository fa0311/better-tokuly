import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { patchVideoEverywhere, videoKeys } from "./api";
import { SaveDialogHost } from "./save-dialog";

/**
 * Runs for as long as someone is signed in. A creator's videos change state
 * on their own — processing finishes or fails, a scheduled video goes live —
 * and the server says so over realtime: the cached cards, the studio lists
 * and the open editor follow without a reload. It also hosts the app's one
 * "save to playlist" dialog, which any video card can open.
 */
export default function VideoBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case !== "video" || frame.body.value.event.case !== "videoUpdated") return;
      const summary = frame.body.value.event.value;
      patchVideoEverywhere(queryClient, summary.id, () => summary);
      // The full video (playback, studio details) and the filtered studio lists are read again.
      void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
      void queryClient.invalidateQueries({ queryKey: videoKeys.studio });
    });
    client.start();
    return offFrame;
  }, [queryClient]);

  return <SaveDialogHost />;
}
