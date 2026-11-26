import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { applyPlansEvent } from "@tokuly/core/plans";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { keys, nameOf } from "./api";

/**
 * Runs while someone is signed in: keeps loaded plans, polls, counters and
 * free statuses current from the realtime connection, and says so at once
 * when a friend answers "あそぼ!" to the person's own status.
 */
export default function PlansBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useTranslation("plans");

  useEffect(() => {
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Whatever changed while the connection was down was not announced.
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: keys.all });
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "plans") return;
      const event = frame.body.value.event;
      applyPlansEvent(queryClient, event);
      if (event.case === "freeAnswer") {
        const text = t("free.answerToast", { name: nameOf(event.value.user) });
        toast(event.value.message ? `${text}「${event.value.message}」` : text, {
          icon: "🙌",
          action: {
            label: t("free.answerToastOpen"),
            onClick: () => void router.navigate({ to: "/plans/free" }),
          },
        });
      }
    });
    client.start();
    return off;
  }, [queryClient, router, t]);

  return null;
}
