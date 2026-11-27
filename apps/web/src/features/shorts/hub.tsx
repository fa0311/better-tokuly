import { Zap } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way into the swipe feed from the hub, the launcher and the quick search. */
export default [
  {
    id: "shorts",
    section: "make",
    order: 25,
    to: "/shorts",
    label: "shorts:hubTile",
    icon: Zap,
    tint: "bg-pink",
    keywords: ["shorts", "short", "ショート", "縦動画", "スワイプ", "swipe", "reels", "tiktok"],
  },
] satisfies HubTile[];
