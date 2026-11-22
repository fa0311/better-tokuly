import { fromDate } from "@tokuly/api";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { interactionClient } from "./data";
import { emitEffect } from "./effects";

let bumping = false;

/**
 * One bump attempt: tells the server "I shook here, now" and waits a few
 * seconds for someone nearby to do the same. Returns whether a bump happened.
 */
export async function tryBump(myId: string | undefined, intensity = 1.5): Promise<boolean> {
  const own = useLocations.getState().own;
  const t = (key: string) => i18n.t(key, { ns: "map" });
  if (!own) {
    toast.error(t("bump.needLocation"));
    return false;
  }
  if (bumping) return false;
  bumping = true;
  const waiting = toast.loading(t("bump.waiting"));
  try {
    // The other person has a few seconds to shake too: ask again until the window closes.
    for (let attempt = 0; attempt < 4; attempt++) {
      const response = await interactionClient().bump({
        location: { lat: own.lat, lng: own.lng },
        accuracyMeters: own.accuracyMeters,
        shakenAt: fromDate(new Date()),
        intensity,
      });
      const bump = response.bumps[0];
      if (bump) {
        const other = bump.participants.find((participant) => participant.id !== myId);
        if (bump.friendRequestCreated)
          toast.success(i18n.t("bump.requestSent", { ns: "map", name: other?.displayName }));
        else if (other)
          emitEffect({
            kind: "bump",
            id: bump.id,
            with: other,
            pairCount: bump.pairCount,
            placeName: bump.placeName,
          });
        return true;
      }
      await new Promise((resolve) => setTimeout(resolve, 1100));
    }
    toast(t("bump.nobody"), { icon: "🤷" });
    return false;
  } catch (error) {
    toast.error(errorMessage(error, { namespace: "map" }));
    return false;
  } finally {
    toast.dismiss(waiting);
    bumping = false;
  }
}

/** Acceleration (m/s², gravity included) beyond which a movement counts as a shake. */
const SHAKE_THRESHOLD = 26;

/**
 * Shaking the phone while the map is open is a bump, like in the original.
 * Only where the browser hands out motion events without asking (Android);
 * everywhere else the bump button does the same.
 */
export function useShakeToBump(): void {
  const myId = useMe()?.user?.id;
  const last = useRef(0);
  useEffect(() => {
    if (typeof DeviceMotionEvent === "undefined" || "requestPermission" in DeviceMotionEvent)
      return;
    const onMotion = (event: DeviceMotionEvent) => {
      const a = event.accelerationIncludingGravity;
      if (!a) return;
      const magnitude = Math.hypot(a.x ?? 0, a.y ?? 0, a.z ?? 0);
      if (
        magnitude < SHAKE_THRESHOLD ||
        Date.now() - last.current < 6000 ||
        document.visibilityState !== "visible"
      )
        return;
      last.current = Date.now();
      void tryBump(myId, magnitude / 9.81);
    };
    window.addEventListener("devicemotion", onMotion);
    return () => window.removeEventListener("devicemotion", onMotion);
  }, [myId]);
}
