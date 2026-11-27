import { Bookmark } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to saved things from the hub. */
export default [
  {
    id: "bookmarks",
    section: "make",
    order: 60,
    to: "/bookmarks",
    label: "engagement:tile",
    icon: Bookmark,
    tint: "bg-sky",
    keywords: ["bookmarks", "saved", "ブックマーク", "保存"],
  },
] satisfies HubTile[];
