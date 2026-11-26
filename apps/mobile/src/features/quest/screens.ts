import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "quest/index", options: () => ({ title: i18n.t("quest:title") }) },
  { name: "quest/badges/index", options: () => ({ title: i18n.t("quest:badges.title") }) },
  // The badge's own name is its title (set by the screen once it is loaded).
  { name: "quest/badges/[key]", options: () => ({ title: i18n.t("quest:badge.title") }) },
] satisfies ScreenEntry[];
