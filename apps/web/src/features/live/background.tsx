import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { onFramesLoading, realtime } from "#/lib/realtime";

/**
 * Runs for as long as someone is signed in: what the live domain tells one
 * person about their own clips and stream (`./frames`).
 */
export default function LiveBackground() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { t } = useTranslation("live");

  useEffect(() => {
    const openClip = (clipId: string) => void navigate({ to: "/clip/$clipId", params: { clipId } });
    const off = onFramesLoading("live", () =>
      import("./frames").then(({ applyOwnLiveEvent }) => ({ event }, frame) => {
        // Topic-scoped events belong to the open stream page, which listens itself.
        if (!frame.topic) applyOwnLiveEvent(queryClient, event, { t, openClip });
      }),
    );
    realtime().start();
    return off;
  }, [queryClient, navigate, t]);

  return null;
}
