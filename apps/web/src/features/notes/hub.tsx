import { Activity, Hash, Paperclip } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to "what my friends did" from the hub. */
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
    id: "note-channels",
    section: "make",
    order: 72,
    to: "/notes/channels",
    label: () => i18n.t("channels:tile"),
    icon: Hash,
    tint: "bg-sky",
    keywords: ["channel", "channels", "チャンネル", "話題"],
  },
  {
    id: "note-clips",
    section: "make",
    order: 74,
    to: "/notes/clips",
    label: () => i18n.t("clips:tile"),
    icon: Paperclip,
    tint: "bg-sun",
    keywords: ["clip", "clips", "クリップ", "まとめ"],
  },
] satisfies HubTile[];
