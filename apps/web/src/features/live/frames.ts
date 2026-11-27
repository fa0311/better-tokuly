import type { QueryClient } from "@tanstack/react-query";
import { live } from "@tokuly/api";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import { liveKeys, patchStream } from "./api";

/**
 * What the live domain tells one person, wherever they are in the app: a clip
 * they cut finished (or failed), and their own stream changed (went on air
 * from OBS, ended, got its archive). Loaded with the first such event.
 */
export function applyOwnLiveEvent(
  queryClient: QueryClient,
  event: live.LiveEvent["event"],
  { t, openClip }: { t: TFunction<"live">; openClip: (shortId: string) => void },
): void {
  if (event.case === "clipUpdated") {
    const clip = event.value;
    queryClient.setQueryData(liveKeys.clip(clip.shortId), clip);
    queryClient.setQueryData(liveKeys.clip(clip.id), clip);
    void queryClient.invalidateQueries({ queryKey: ["live", "clip", "list"] });
    if (clip.status === live.ClipStatus.READY)
      toast.success(t("clip.readyToast", { title: clip.title }), {
        action: { label: t("clip.open"), onClick: () => openClip(clip.shortId) },
      });
    else if (clip.status === live.ClipStatus.FAILED)
      toast.error(t("clip.failedToast", { title: clip.title }));
  } else if (event.case === "streamUpdated") {
    const summary = event.value;
    patchStream(queryClient, summary.id, (stream) => ({ ...stream, summary }));
    void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
  }
}
