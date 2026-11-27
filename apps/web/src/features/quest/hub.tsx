import { Trophy } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to missions, badges, streaks and rankings from the hub. */
export default [
  {
    id: "quest",
    section: "play",
    order: 10,
    to: "/quest",
    label: "quest:title",
    icon: Trophy,
    tint: "bg-sun",
    badge: "quest",
    keywords: [
      "quest",
      "mission",
      "badge",
      "level",
      "streak",
      "ranking",
      "ミッション",
      "バッジ",
      "レベル",
      "ストリーク",
      "ランキング",
      "称号",
    ],
    card: () => import("./hub-card"),
  },
] satisfies HubTile[];
