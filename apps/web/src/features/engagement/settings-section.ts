import { SmilePlus } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the reaction settings: custom and sensitive emoji on one's own posts. */
export default {
  id: "reactions",
  order: 46,
  to: "/settings/reactions",
  icon: SmilePlus,
  tint: "bg-sun text-[#17122b]",
  title: "engagement:reactionSettings.rowTitle",
  hint: "engagement:reactionSettings.rowHint",
} satisfies SettingsSection;
