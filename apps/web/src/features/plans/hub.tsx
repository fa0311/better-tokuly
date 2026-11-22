import { CalendarHeart } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to plans ("あそぶ") from the hub. */
export default [
  {
    id: "plans",
    section: "play",
    order: 20,
    to: "/plans",
    label: () => i18n.t("plans:tile"),
    icon: CalendarHeart,
    tint: "bg-lime",
    badge: "plans",
    keywords: ["plans", "calendar", "予定", "ひま", "カレンダー", "カウントダウン"],
  },
] satisfies HubTile[];
