import { Smile } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to stamps from the hub. */
export default [
  {
    id: "stamps",
    section: "make",
    order: 10,
    to: "/stamps",
    label: () => i18n.t("common:nav.stamps"),
    icon: Smile,
    tint: "bg-sun",
    keywords: ["stamps", "stickers", "スタンプ"],
  },
] satisfies HubTile[];
