import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { spaces } from "@tokuly/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { useMe } from "#/stores/session";
import { keys } from "./api";
import { applySpacesEvent, useSpaceTopic } from "./live";
import { MiniPlayer } from "./mini-player";
import { onStage } from "./model";
import { type ExitReason, putOutside, roleChanged, useAudioSession } from "./session";

const Leave = spaces.SpaceLeaveReason;

/** Why the server says the person is outside, in this app's words. */
function exitReasonOf(reason: spaces.SpaceLeaveReason): ExitReason {
  if (reason === Leave.REMOVED) return "removed";
  if (reason === Leave.BANNED) return "banned";
  if (reason === Leave.ENDED) return "ended";
  if (reason === Leave.DISCONNECTED) return "lost";
  // They left with another device, or entered another room there.
  return "elsewhere";
}

/**
 * Runs while someone is signed in: keeps loaded rooms and hangouts current
 * from the realtime connection, tells the audio session what the server
 * decided about the person (promoted, muted, removed, the room over), and
 * shows the mini player of the room they are inside.
 */
export default function SpacesBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useTranslation("spaces");
  const myId = useMe()?.user?.id ?? "";
  const activeSpaceId = useAudioSession((state) =>
    state.active?.kind === "space" ? state.active.id : undefined,
  );
  // The room keeps being followed while the person is elsewhere in the app.
  useSpaceTopic(activeSpaceId);

  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Whatever changed while the connection was down was not announced.
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: keys.all });
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "spaces") return;
      const event = frame.body.value.event;
      const active = useAudioSession.getState().active;
      const onPage = (path: string) => router.state.location.pathname.replace(/\/+$/, "") === path;

      switch (event.case) {
        case "participantUpdated": {
          const { spaceId, participant } = event.value;
          if (participant?.user?.id !== myId) break;
          const before = queryClient.getQueryData<spaces.Space | null>(keys.space(spaceId));
          const was = onStage(before?.viewer?.role ?? spaces.SpaceRole.LISTENER);
          const now = onStage(participant.role);
          if (was === now) break;
          roleChanged(queryClient, spaceId, participant.role);
          if (active?.kind === "space" && active.id === spaceId)
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
        case "hangoutPresence":
          // Out of the voice room without having left here: another device did, or the connection was given up on.
          if (event.value.user?.id === myId && !event.value.inside)
            putOutside("hangout", event.value.hangoutId, "elsewhere");
          break;
        default:
          break;
      }
      applySpacesEvent(queryClient, event, myId);
    });
    client.start();
    return off;
  }, [queryClient, router, t, myId]);

  // Being put outside is said wherever the person is; the venue's page then shows the details.
  useEffect(
    () =>
      useAudioSession.subscribe((state, previous) => {
        const exit = state.exit;
        if (!exit || (previous.exit?.kind === exit.kind && previous.exit.id === exit.id)) return;
        toast(t(`exit.${exit.reason}.title`), {
          description: t(`exit.${exit.reason}.body`),
        });
      }),
    [t],
  );

  return <MiniPlayer />;
}
