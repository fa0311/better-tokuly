import { errorReason, fromDate } from "@tokuly/api";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { interactionClient } from "./data";
import { emitEffect } from "./effects";
import {
  type ClaimProblem,
  claimProblem,
  PRECISE_ENOUGH_METERS,
  reportFreshFix,
} from "./own-location";

let bumping = false;

/** A refused position, with a retry. */
function showClaimProblem(problem: ClaimProblem, retry: () => void): void {
  toast.error(i18n.t(problem, { ns: "errors" }), {
    action: { label: i18n.t("actions.retry"), onClick: retry },
  });
}

/**
 * One bump attempt: tells the server "I shook here, now" and waits a few
 * seconds for someone nearby to do the same. Returns whether a bump happened.
 */
export async function tryBump(myId: string | undefined, intensity = 1.5): Promise<boolean> {
  const t = (key: string) => i18n.t(key, { ns: "map" });
  if (!useLocations.getState().own) {
    toast.error(t("bump.needLocation"));
    return false;
  }
  if (bumping) return false;
  bumping = true;
  const waiting = toast.loading(t("bump.waiting"));
  try {
    // The server takes a shake only where our own latest fix puts us: send a fresh one first.
    const own = (await reportFreshFix()) ?? useLocations.getState().own;
    if (!own) {
      toast.error(t("bump.needLocation"));
      return false;
    }
    // Approximate location (an OS setting) is refused anyway: say so without asking.
    if (own.accuracyMeters > PRECISE_ENOUGH_METERS) {
      showClaimProblem("LOCATION_TOO_COARSE", () => void tryBump(myId, intensity));
      return false;
    }
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
    const problem = claimProblem(errorReason(error));
    if (problem) showClaimProblem(problem, () => void tryBump(myId, intensity));
    else toast.error(errorMessage(error, { namespace: "map" }));
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
