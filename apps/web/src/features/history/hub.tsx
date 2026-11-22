import { CalendarDays } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the location diary from the hub. */
export default [
  {
    id: "history",
    section: "out",
    order: 20,
    to: "/history",
    label: () => i18n.t("history:tile"),
    icon: CalendarDays,
    tint: "bg-sun",
    keywords: ["history", "diary", "りれき", "履歴", "日記"],
  },
] satisfies HubTile[];
