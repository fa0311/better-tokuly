import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { spaces } from "@tokuly/api";
import { rereadOnWelcome } from "@tokuly/core/query";
import { ELSEWHERE_WINDOW_MS, exitReasonOf, onStage, spacesKeys } from "@tokuly/core/spaces";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { useMe } from "#/stores/session";

import { EndCallPrompt } from "./end-call-prompt";
import { applySpacesEvent, useSpaceTopic } from "./live";
import { MiniPlayer } from "./mini-player";
import { putOutside, roleChanged, useAudioSession } from "./session";

/**
 * Runs while someone is signed in: keeps loaded rooms and hangouts current
 * from the realtime connection, tells the audio session what the server
 * decided about the person (promoted, muted, removed, the room over), and
 * shows the mini player of the room they are inside.
 */
export default function SpacesBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { i18n } = useTranslation("spaces");
  const myId = useMe()?.user?.id ?? "";
  const activeSpaceId = useAudioSession((state) =>
    state.active?.kind === "space" ? state.active.id : undefined,
  );
  // The room keeps being followed while the person is elsewhere in the app.
  useSpaceTopic(activeSpaceId);

  useEffect(() => {
    if (!myId) return;
    const t = i18n.getFixedT(null, "spaces");
    const client = realtime();
    /** When this person was last seen entering a room or hangout other than this session's. */
    let enteredElsewhereAt = 0;
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "spaces") return;
      const event = frame.body.value.event;
      const active = useAudioSession.getState().active;
      const onPage = (path: string) => router.state.location.pathname.replace(/\/+$/, "") === path;

      switch (event.case) {
        case "participantUpdated": {
          const { spaceId, participant } = event.value;
          if (participant?.user?.id !== myId) break;
          // Inside another room on another device: this one is about to be left.
          if (active?.kind !== "space" || active.id !== spaceId) {
            enteredElsewhereAt = Date.now();
            break;
          }
          // The role this session holds is the one to compare with; caches come and go.
          const was = onStage(active.role);
          const now = onStage(participant.role);
          if (was === now) break;
          roleChanged(queryClient, spaceId, participant.role);
          toast(t(now ? "toast.youAreOnStage" : "toast.backToAudience"), {
            icon: now ? "🎤" : "🪑",
          });
          break;
        }
        case "participantLeft":
          if (event.value.userId === myId)
            putOutside("space", event.value.spaceId, exitReasonOf(event.value.reason));
          break;
        case "spaceUpdated": {
          const space = event.value.space;
          if (
            space &&
            (space.state === spaces.SpaceState.ENDED || space.state === spaces.SpaceState.CANCELLED)
          )
            putOutside("space", space.id, "ended");
          break;
        }
        case "speakerMuted":
          if (event.value.userId === myId && event.value.mutedById !== myId)
            toast(t("toast.mutedByModerator"), { icon: "🔇" });
          break;
        case "stageInvitation": {
          const { spaceId, invited, invitedBy } = event.value;
          // On the room's page the invitation is a banner with its buttons.
          if (!invited || onPage(`/spaces/${spaceId}`)) break;
          toast(t("toast.stageInvite", { name: invitedBy?.displayName || invitedBy?.handle }), {
            icon: "🎤",
            action: {
              label: t("toast.open"),
              onClick: () => void router.navigate({ to: "/spaces/$spaceId", params: { spaceId } }),
            },
          });
          break;
        }
        case "hangoutRemoved":
          putOutside("hangout", event.value.hangoutId, "closed");
          break;
        case "hangoutPresence": {
          if (event.value.user?.id !== myId) break;
          if (event.value.inside) {
            if (active?.kind !== "hangout" || active.id !== event.value.hangoutId)
              enteredElsewhereAt = Date.now();
            break;
          }
          // Out of the voice room without having left here: the server gave the connection up,
          // unless this person was just seen entering somewhere else.
          const elsewhere = Date.now() - enteredElsewhereAt < ELSEWHERE_WINDOW_MS;
          putOutside("hangout", event.value.hangoutId, elsewhere ? "elsewhere" : "lost");
          break;
        }
        default:
          break;
      }
      applySpacesEvent(queryClient, event, myId);
    });
    // What changed while the connection was down was not announced: the lists on screen
    // (hangouts, rooms live and upcoming, friends in rooms) are read again. A followed room reads
    // itself and its participants again when its topic is back (`useSpaceTopic`).
    const stop = rereadOnWelcome(
      queryClient,
      client,
      [spacesKeys.all],
      (key) =>
        (key[1] === "space" || key[1] === "participants") &&
        client.isSubscribed(`space:${String(key[2])}`),
    );
    client.start();
    return () => {
      off();
      stop();
    };
  }, [queryClient, router, i18n, myId]);

  // Being put outside is said wherever the person is; the venue's page then shows the details.
  useEffect(() => {
    const t = i18n.getFixedT(null, "spaces");
    return useAudioSession.subscribe((state, previous) => {
      const exit = state.exit;
      if (!exit || (previous.exit?.kind === exit.kind && previous.exit.id === exit.id)) return;
      toast(t(`exit.${exit.reason}.title`), {
        description: t(`exit.${exit.reason}.body`),
      });
    });
  }, [i18n]);

  return (
    <>
      <MiniPlayer />
      <EndCallPrompt />
    </>
  );
}
