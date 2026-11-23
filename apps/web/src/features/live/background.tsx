import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { live } from "@tokuly/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { liveKeys, patchStream } from "./api";

/**
 * Runs for as long as someone is signed in. What the live domain tells one
 * person, wherever they are in the app: a clip they cut finished (or failed),
 * and their own stream changed (went on air from OBS, ended, got its archive).
 */
export default function LiveBackground() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { t } = useTranslation("live");

  useEffect(() => {
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      // Topic-scoped events belong to the open stream page, which listens itself.
      if (frame.body.case !== "live" || frame.topic) return;
      const event = frame.body.value.event;
      if (event.case === "clipUpdated") {
        const clip = event.value;
        queryClient.setQueryData(liveKeys.clip(clip.shortId), clip);
        queryClient.setQueryData(liveKeys.clip(clip.id), clip);
        void queryClient.invalidateQueries({ queryKey: ["live", "clip", "list"] });
        if (clip.status === live.ClipStatus.READY)
          toast.success(t("clip.readyToast", { title: clip.title }), {
            action: {
              label: t("clip.open"),
              onClick: () =>
                void navigate({ to: "/clip/$clipId", params: { clipId: clip.shortId } }),
            },
          });
        else if (clip.status === live.ClipStatus.FAILED)
          toast.error(t("clip.failedToast", { title: clip.title }));
      } else if (event.case === "streamUpdated") {
        const summary = event.value;
        patchStream(queryClient, summary.id, (stream) => ({ ...stream, summary }));
        void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
      }
    });
    client.start();
    return offFrame;
  }, [queryClient, navigate, t]);

  return null;
}
