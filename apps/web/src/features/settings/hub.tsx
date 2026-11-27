import { Settings, Users } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the settings from the hub. */
export default [
  {
    id: "settings",
    section: "me",
    order: 30,
    to: "/settings",
    label: "common:nav.settings",
    icon: Settings,
    tint: "bg-mint",
    keywords: ["settings", "設定", "せってい"],
  },
  {
    id: "accounts",
    section: "me",
    order: 35,
    to: "/settings/accounts",
    label: "accounts:tile",
    icon: Users,
    tint: "bg-sky",
    keywords: ["アカウント切り替え", "裏垢", "switch account", "accounts"],
  },
] satisfies HubTile[];
