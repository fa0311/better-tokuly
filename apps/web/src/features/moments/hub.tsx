import { Zap } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to moments ("あそぶ") from the hub. */
export default [
  {
    id: "moments",
    section: "play",
    order: 5,
    to: "/moments",
    label: "moments:tile",
    icon: Zap,
    tint: "bg-sun",
    keywords: ["moments", "realmoji", "いまの瞬間", "瞬間", "BeReal", "カメラ"],
  },
] satisfies HubTile[];
