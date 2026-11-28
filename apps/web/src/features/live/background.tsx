import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { useTranslation } from "#/lib/use-translation";

/**
 * Runs for as long as someone is signed in: what the live domain tells one
 * person about their own clips and stream (`./frames`).
 */
export default function LiveBackground() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { i18n } = useTranslation("live");

  useEffect(() => {
    // The hook's `t` is new whenever any namespace loads; resubscribing then would drop the frames
    // waiting for their handler. This one stays and reads the current language.
    const t = i18n.getFixedT(null, "live");
    const openClip = (clipId: string) => void navigate({ to: "/clip/$clipId", params: { clipId } });
    const off = onFramesLoading("live", () =>
      import("./frames").then(({ applyOwnLiveEvent }) => ({ event }, frame) => {
        // Topic-scoped events belong to the open stream page, which listens itself.
        if (!frame.topic) applyOwnLiveEvent(queryClient, event, { t, openClip });
      }),
    );
    realtime().start();
    return off;
  }, [queryClient, navigate, i18n]);

  return null;
}
