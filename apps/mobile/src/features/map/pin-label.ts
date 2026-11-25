import { geo, toMillis } from "@tokuly/api";
import { compactAge, compactDuration, kmh } from "@tokuly/core/format";
import { isBlurred, isMoving, pinState, stayMillis } from "@tokuly/core/map";
import i18n from "#/lib/i18n";

export const ACTIVITY_EMOJI: Record<number, string> = {
  [geo.Activity.WALKING]: "🚶",
  [geo.Activity.RUNNING]: "🏃",
  [geo.Activity.CYCLING]: "🚲",
  [geo.Activity.DRIVING]: "🚗",
  [geo.Activity.TRAIN]: "🚃",
  [geo.Activity.FLYING]: "✈️",
  [geo.Activity.BOAT]: "⛴️",
};

export const ACTIVITY_KEYS: Record<number, string> = {
  [geo.Activity.STILL]: "still",
  [geo.Activity.WALKING]: "walking",
  [geo.Activity.RUNNING]: "running",
  [geo.Activity.CYCLING]: "cycling",
  [geo.Activity.DRIVING]: "driving",
  [geo.Activity.TRAIN]: "train",
  [geo.Activity.FLYING]: "flying",
  [geo.Activity.BOAT]: "boat",
};

/** The pill under a friend's pin: paused, roughly here, on the move, how long here, or how fresh. */
export function pinLabel(location: geo.FriendLocation, now: number): string {
  const t = (key: string) => i18n.t(key, { ns: "map" });
  if (pinState(location, now) === "paused") return t("pin.paused");
  if (isBlurred(location)) return t("pin.around");
  if (isMoving(location, now))
    return `${ACTIVITY_EMOJI[location.activity] ?? ""} ${kmh(location.speedMps)} km/h`.trim();
  const stay = stayMillis(location, now);
  if (stay !== null) return compactDuration(i18n.language, stay);
  return compactAge(i18n, toMillis(location.recordedAt), now);
}
