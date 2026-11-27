import { CalendarDays } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the location diary from the hub. */
export default [
  {
    id: "history",
    section: "out",
    order: 20,
    to: "/history",
    label: "history:tile",
    icon: CalendarDays,
    tint: "bg-sun",
    keywords: ["history", "diary", "りれき", "履歴", "日記"],
  },
] satisfies HubTile[];
