import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "stamps/index", options: () => ({ title: i18n.t("stamps:title") }) },
  { name: "stamps/library", options: () => ({ title: i18n.t("stamps:library.title") }) },
  { name: "stamps/[packId]", options: () => ({ title: i18n.t("stamps:pack.stamps") }) },
] satisfies ScreenEntry[];
