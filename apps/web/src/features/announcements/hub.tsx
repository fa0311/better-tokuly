import { Megaphone } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the update log from the hub. */
export default [
  {
    id: "announcements",
    section: "me",
    order: 40,
    to: "/announcements",
    label: "common:nav.announcements",
    icon: Megaphone,
    tint: "bg-sun",
    keywords: ["announcements", "updates", "アップデート"],
  },
] satisfies HubTile[];
