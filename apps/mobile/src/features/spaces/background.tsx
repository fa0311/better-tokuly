import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import { applySpacesEvent, onStage, spacesKeys } from "@tokuly/core/spaces";
import { useEffect } from "react";
import { useMe } from "#/features/session/store";
import { realtime } from "#/lib/realtime";
import { emitBurst, isOwnBurst } from "./reactions";
import { type ExitReason, putOutside, roleChanged, useAudioSession } from "./session";
import { useSpaceTopic } from "./topic";

const Leave = spaces.SpaceLeaveReason;

/** How recent an entry elsewhere must be to explain being out of this venue. */
const ELSEWHERE_WINDOW_MS = 15_000;

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
 * from the realtime connection, draws the emoji bursts of others, and tells
 * the audio session what the server decided about the person (promoted,
 * removed, the room over). The room they are inside stays followed wherever
 * they are in the app.
 */
export default function SpacesBackground() {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  const activeSpaceId = useAudioSession((state) =>
    state.active?.kind === "space" ? state.active.id : undefined,
  );
  useSpaceTopic(activeSpaceId);

  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    /** When this person was last seen entering a venue other than this session's. */
    let enteredElsewhereAt = 0;
    let welcomed = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // What changed while the connection was down was not announced: the lists are read again.
        if (welcomed) {
          void queryClient.invalidateQueries({ queryKey: spacesKeys.lists });
          void queryClient.invalidateQueries({ queryKey: spacesKeys.hangouts });
          void queryClient.invalidateQueries({ queryKey: spacesKeys.friendsInRooms });
        }
        welcomed = true;
        return;
      }
      if (frame.body.case !== "spaces") return;
      const event = frame.body.value.event;
      const active = useAudioSession.getState().active;
      switch (event.case) {
        case "reaction":
          if (!isOwnBurst(event.value.nonce))
            emitBurst({ spaceId: event.value.spaceId, emoji: event.value.emoji });
          return;
        case "participantUpdated": {
          const { spaceId, participant } = event.value;
          if (participant?.user?.id !== myId) break;
          // Inside another room on another device: this one is about to be left.
          if (active?.kind !== "space" || active.id !== spaceId) {
            enteredElsewhereAt = Date.now();
            break;
          }
          if (onStage(active.role) !== onStage(participant.role))
            roleChanged(queryClient, spaceId, participant.role);
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
    client.start();
    return off;
  }, [queryClient, myId]);

  return null;
}
