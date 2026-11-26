import { useQueryClient } from "@tanstack/react-query";
import { patchVideoEverywhere, videoKeys } from "@tokuly/core/video";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { SaveSheetHost } from "./save-sheet";

/**
 * Runs while someone is signed in. A video changes on its own — processing
 * finishes, a scheduled video goes live — and the server says so over
 * realtime: the cached cards and the open watch screen follow. It also
 * hosts the app's one "save to…" sheet, which any video card can open.
 */
export default function VideoBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "video" || frame.body.value.event.case !== "videoUpdated") return;
      const summary = frame.body.value.event.value;
      patchVideoEverywhere(queryClient, summary.id, () => summary);
      void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
    });
    client.start();
    return off;
  }, [queryClient]);
  return <SaveSheetHost />;
}
