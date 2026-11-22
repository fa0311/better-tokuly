import { Bookmark } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to saved things from the hub. */
export default [
  {
    id: "bookmarks",
    section: "make",
    order: 60,
    to: "/bookmarks",
    label: () => i18n.t("engagement:tile"),
    icon: Bookmark,
    tint: "bg-sky",
    keywords: ["bookmarks", "saved", "ブックマーク", "保存"],
  },
] satisfies HubTile[];
