import { SmilePlus } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to custom emoji from the hub. */
export default [
  {
    id: "emoji",
    section: "make",
    order: 20,
    to: "/emoji",
    label: "emoji:tile",
    icon: SmilePlus,
    tint: "bg-mint",
    keywords: ["emoji", "絵文字", "えもじ"],
  },
] satisfies HubTile[];
