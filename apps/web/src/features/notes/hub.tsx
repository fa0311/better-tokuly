import { Activity, ListChecks, Radio } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The ways in from the hub: "what my friends did", note lists and antennas. */
export default [
  {
    id: "activity",
    section: "make",
    order: 70,
    to: "/activity",
    label: () => i18n.t("notes:activityTile"),
    icon: Activity,
    tint: "bg-pink",
    keywords: ["activity", "log", "ログ", "アクティビティ"],
  },
  {
    id: "note-lists",
    section: "make",
    order: 72,
    to: "/notes/lists",
    label: () => i18n.t("notelists:tile"),
    icon: ListChecks,
    tint: "bg-sky",
    keywords: ["list", "lists", "リスト", "ノートリスト", "タイムライン"],
  },
  {
    id: "note-antennas",
    section: "make",
    order: 74,
    to: "/notes/antennas",
    label: () => i18n.t("notelists:antennaTile"),
    icon: Radio,
    tint: "bg-sun",
    keywords: ["antenna", "アンテナ", "keyword", "キーワード", "検索"],
  },
] satisfies HubTile[];
