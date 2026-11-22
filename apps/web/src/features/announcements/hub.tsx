import { Megaphone } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the update log from the hub. */
export default [
  {
    id: "announcements",
    section: "me",
    order: 40,
    to: "/announcements",
    label: () => i18n.t("common:nav.announcements"),
    icon: Megaphone,
    tint: "bg-sun",
    keywords: ["announcements", "updates", "アップデート"],
  },
] satisfies HubTile[];
