import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { momentKeys } from "@tokuly/core/moments/keys";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { onFramesLoading, realtime } from "#/lib/realtime";

/**
 * Runs while someone is signed in: keeps today, friends' moments and the
 * map current from the realtime connection, and says 「⚡いまの瞬間を撮ろう!」
 * the moment the prompt arrives, with the camera one tap away.
 */
export default function MomentsBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useTranslation("moments");

  useEffect(() => {
    const client = realtime();
    let connectedBefore = false;
    const offWelcome = client.onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      // Whatever happened while the connection was down was not announced.
      if (connectedBefore) void queryClient.invalidateQueries({ queryKey: momentKeys.all });
      connectedBefore = true;
    });
    const offEvents = onFramesLoading("moments", () =>
      import("@tokuly/core/moments").then(({ applyMomentsEvent }) => ({ event }) => {
        applyMomentsEvent(queryClient, event);
        if (event.case !== "prompt" || router.state.location.pathname === "/moments/camera") return;
        toast(t("today.openTitle"), {
          id: "moments-prompt",
          duration: 120_000,
          action: {
            label: t("today.take"),
            onClick: () => void router.navigate({ to: "/moments/camera" }),
          },
        });
      }),
    );
    client.start();
    return () => {
      offWelcome();
      offEvents();
    };
  }, [queryClient, router, t]);

  return null;
}
