import { MapPinned } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to places from the hub. */
export default [
  {
    id: "places",
    section: "out",
    order: 30,
    to: "/places",
    label: () => i18n.t("places:tile"),
    icon: MapPinned,
    tint: "bg-mint",
    keywords: ["places", "checkin", "スポット", "場所", "チェックイン"],
  },
] satisfies HubTile[];
