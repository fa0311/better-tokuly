import { Bell } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the notification inbox from the hub. */
export default [
  {
    id: "notifications",
    section: "me",
    order: 10,
    to: "/notifications",
    label: () => i18n.t("common:nav.notifications"),
    icon: Bell,
    tint: "bg-pink",
    badge: "notifications",
    keywords: ["notifications", "お知らせ", "通知"],
  },
] satisfies HubTile[];
