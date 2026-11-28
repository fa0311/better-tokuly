import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { planKeys } from "@tokuly/core/plans/keys";
import { useEffect } from "react";
import { toast } from "sonner";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { useTranslation } from "#/lib/use-translation";

/**
 * Runs while someone is signed in: keeps loaded plans, polls, counters and
 * free statuses current from the realtime connection, and says so at once
 * when a friend answers "あそぼ!" to the person's own status.
 */
export default function PlansBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { i18n } = useTranslation("plans");

  useEffect(() => {
    // The hook's `t` is new whenever any namespace loads; resubscribing then would drop the frames
    // waiting for their handler. This one stays and reads the current language.
    const t = i18n.getFixedT(null, "plans");
    const client = realtime();
    let connectedBefore = false;
    const offWelcome = client.onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      // Whatever changed while the connection was down was not announced.
      if (connectedBefore) void queryClient.invalidateQueries({ queryKey: planKeys.all });
      connectedBefore = true;
    });
    const offEvents = onFramesLoading("plans", () =>
      import("./frames").then(({ applyPlansEvent, nameOf }) => ({ event }) => {
        applyPlansEvent(queryClient, event);
        if (event.case !== "freeAnswer") return;
        const text = t("free.answerToast", { name: nameOf(event.value.user) });
        toast(event.value.message ? `${text}「${event.value.message}」` : text, {
          icon: "🙌",
          action: {
            label: t("free.answerToastOpen"),
            onClick: () => void router.navigate({ to: "/plans/free" }),
          },
        });
      }),
    );
    client.start();
    return () => {
      offWelcome();
      offEvents();
    };
  }, [queryClient, router, i18n]);

  return null;
}
