import { MapPinned } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to places from the hub. */
export default [
  {
    id: "places",
    section: "out",
    order: 30,
    to: "/places",
    label: "places:tile",
    icon: MapPinned,
    tint: "bg-mint",
    keywords: ["places", "checkin", "スポット", "場所", "チェックイン"],
  },
] satisfies HubTile[];
