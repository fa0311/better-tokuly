import { Newspaper } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the news from the hub. */
export default [
  {
    id: "news",
    section: "make",
    order: 80,
    to: "/news",
    label: () => i18n.t("common:nav.news"),
    icon: Newspaper,
    tint: "bg-coral",
    keywords: ["news", "ニュース"],
  },
] satisfies HubTile[];
