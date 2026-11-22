import { Users } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the friends list from the hub. */
export default [
  {
    id: "friends",
    section: "play",
    order: 10,
    to: "/friends",
    label: () => i18n.t("common:nav.friends"),
    icon: Users,
    tint: "bg-sky",
    badge: "friends",
    keywords: ["friends", "ともだち", "友達", "フレンド"],
  },
] satisfies HubTile[];
