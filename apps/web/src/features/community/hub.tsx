import { UsersRound } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to communities from the hub, the launcher and the quick search. */
export default [
  {
    id: "community",
    section: "play",
    order: 30,
    to: "/communities",
    label: () => i18n.t("community:title"),
    icon: UsersRound,
    tint: "bg-mint",
    badge: "community",
    keywords: [
      "community",
      "communities",
      "group",
      "club",
      "board",
      "コミュニティ",
      "グループ",
      "サークル",
      "部活",
      "掲示板",
      "スレッド",
      "イベント",
    ],
  },
] satisfies HubTile[];
