import { Bell } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the notification inbox from the hub. */
export default [
  {
    id: "notifications",
    section: "me",
    order: 10,
    to: "/notifications",
    label: "common:nav.notifications",
    icon: Bell,
    tint: "bg-pink",
    badge: "notifications",
    keywords: ["notifications", "お知らせ", "通知"],
  },
] satisfies HubTile[];
