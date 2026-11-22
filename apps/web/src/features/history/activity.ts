import { geo } from "@tokuly/api";

/** How each way of travelling is shown: its emoji and its name in the strings (`activity.<key>`). */
const ACTIVITIES: Record<number, { key: string; emoji: string }> = {
  [geo.Activity.STILL]: { key: "still", emoji: "🧍" },
  [geo.Activity.WALKING]: { key: "walking", emoji: "🚶" },
  [geo.Activity.RUNNING]: { key: "running", emoji: "🏃" },
  [geo.Activity.CYCLING]: { key: "cycling", emoji: "🚲" },
  [geo.Activity.DRIVING]: { key: "driving", emoji: "🚗" },
  [geo.Activity.TRAIN]: { key: "train", emoji: "🚃" },
  [geo.Activity.FLYING]: { key: "flying", emoji: "✈️" },
  [geo.Activity.BOAT]: { key: "boat", emoji: "⛴️" },
};

export const activityMeta = (activity: geo.Activity) =>
  ACTIVITIES[activity] ?? { key: "moving", emoji: "➡️" };
