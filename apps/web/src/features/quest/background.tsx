import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { rereadOnWelcome } from "@tokuly/core/query";
import { questKeys } from "@tokuly/core/quest/keys";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { clearReveals, RevealLayer } from "./celebrate";

type QuestCelebrations = typeof import("./frames").questCelebrations;

/**
 * Runs while someone is signed in: XP, level-ups, badges, finished missions
 * and streak changes arrive as realtime events and are celebrated
 * (`./frames`) — a quiet toast at the low end, a card with confetti in the
 * middle, the whole screen at the top. Nothing here takes input away from
 * the page; several moments queue.
 */
export default function QuestBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { i18n } = useTranslation("quest");

  useEffect(() => {
    // The hook's `t` is new whenever any namespace loads; resubscribing then would drop the frames
    // waiting for their handler. This one stays and reads the current language.
    const t = i18n.getFixedT(null, "quest");
    const client = realtime();
    // Pushes sent before the connection was up, or while it was down, are lost: read again.
    const offWelcome = rereadOnWelcome(queryClient, client, [questKeys.all]);
    let celebrations: ReturnType<QuestCelebrations> | null = null;
    const offEvents = onFramesLoading("quest", () =>
      import("./frames").then(({ questCelebrations }) => {
        celebrations = questCelebrations({ queryClient, router, t, language: () => i18n.language });
        return celebrations.apply;
      }),
    );
    client.start();

    // The cache belongs to one account: forget it when that account leaves.
    let userId = signedInUserId(useSessionStore.getState());
    const offSession = useSessionStore.subscribe((session) => {
      const next = signedInUserId(session);
      if (next === userId) return;
      if (userId) {
        queryClient.removeQueries({ queryKey: questKeys.all });
        clearReveals();
        celebrations?.forget();
      }
      userId = next;
    });

    return () => {
      offWelcome();
      offEvents();
      offSession();
    };
  }, [queryClient, router, i18n]);

  return <RevealLayer />;
}

function signedInUserId(session: ReturnType<typeof useSessionStore.getState>): string | undefined {
  return session.state.status === "signedIn" ? session.state.me.user?.id : undefined;
}
