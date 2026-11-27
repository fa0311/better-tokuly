import { Star, Users } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The ways in from the hub: the friends list, and the circles (親しい友達 and custom lists). */
export default [
  {
    id: "friends",
    section: "play",
    order: 10,
    to: "/friends",
    label: "common:nav.friends",
    icon: Users,
    tint: "bg-sky",
    badge: "friends",
    keywords: ["friends", "ともだち", "友達", "フレンド", "フォロー", "follow", "フォロワー"],
  },
  {
    id: "circles",
    section: "me",
    order: 15,
    to: "/circles",
    label: "circles:tile",
    icon: Star,
    tint: "bg-mint",
    keywords: ["circles", "close friends", "親しい友達", "リスト", "list", "サークル"],
  },
] satisfies HubTile[];
