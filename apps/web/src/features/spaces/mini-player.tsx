import { useLocation } from "@tanstack/react-router";
import { Suspense } from "react";
import { onDemand } from "#/components/on-demand";
import { type ActiveVenue, useAudioSession } from "./session";

const Pill = onDemand(() => import("./mini-pill").then((module) => module.Pill));

/** The page of a venue: where the full view of what is playing lives. */
function pathOf(venue: Pick<ActiveVenue, "kind" | "id">): string {
  return venue.kind === "space" ? `/spaces/${venue.id}` : `/spaces/hangouts/${venue.id}`;
}

/**
 * The room the person is inside, as a pill that follows them around the app:
 * what is playing, whether someone talks, mute and leave. On the room's own
 * page the page itself is the player.
 */
export function MiniPlayer() {
  const active = useAudioSession((state) => state.active);
  const pathname = useLocation({ select: (location) => location.pathname });
  if (!active || pathname.replace(/\/+$/, "") === pathOf(active)) return null;
  return (
    <Suspense fallback={null}>
      <Pill active={active} />
    </Suspense>
  );
}
