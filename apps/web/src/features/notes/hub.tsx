import { Activity } from "lucide-react";
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
] satisfies HubTile[];
