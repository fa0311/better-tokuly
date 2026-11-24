import { Settings, Users } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the settings from the hub. */
export default [
  {
    id: "settings",
    section: "me",
    order: 30,
    to: "/settings",
    label: () => i18n.t("common:nav.settings"),
    icon: Settings,
    tint: "bg-mint",
    keywords: ["settings", "設定", "せってい"],
  },
  {
    id: "accounts",
    section: "me",
    order: 35,
    to: "/settings/accounts",
    label: () => i18n.t("accounts:tile"),
    icon: Users,
    tint: "bg-sky",
    keywords: ["アカウント切り替え", "裏垢", "switch account", "accounts"],
  },
] satisfies HubTile[];
