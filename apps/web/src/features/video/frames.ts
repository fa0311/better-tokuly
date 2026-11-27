import type { QueryClient } from "@tanstack/react-query";
import type { video } from "@tokuly/api";
import { patchVideoEverywhere, videoKeys } from "@tokuly/core/video";

/**
 * A creator's video changed state on its own (processing finished or failed, a scheduled video
 * went live): the cached cards follow, and the full video and the studio lists are read again.
 * `background.tsx` loads this with the first video event.
 */
export function applyVideoEvent(queryClient: QueryClient, event: video.VideoEvent["event"]): void {
  if (event.case !== "videoUpdated") return;
  const summary = event.value;
  patchVideoEverywhere(queryClient, summary.id, () => summary);
  void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
  void queryClient.invalidateQueries({ queryKey: videoKeys.studio });
}
