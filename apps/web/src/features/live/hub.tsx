import { Radio, Video } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The live tiles of the hub: watching, and going live. */
export default [
  {
    id: "live",
    section: "make",
    order: 20,
    to: "/live",
    label: "live:tiles.watch",
    icon: Radio,
    tint: "bg-coral",
    badge: "live",
    keywords: ["live", "stream", "ライブ", "配信", "生放送", "クリップ", "clip"],
  },
  {
    id: "live-studio",
    section: "make",
    order: 32,
    to: "/studio/live",
    label: "live:tiles.studio",
    icon: Video,
    tint: "bg-pink",
    keywords: ["go live", "obs", "overlay", "配信する", "ライブ配信", "オーバーレイ"],
  },
] satisfies HubTile[];
