import { Footprints } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the scratch map from the hub. */
export default [
  {
    id: "footprints",
    section: "out",
    order: 10,
    to: "/footprints",
    label: "footprints:tile",
    icon: Footprints,
    tint: "bg-lime",
    keywords: ["footprints", "あしあと", "足跡"],
  },
] satisfies HubTile[];
