import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "places/index", options: () => ({ title: i18n.t("places:title") }) },
  { name: "places/[id]/index", options: () => ({ title: i18n.t("places:title") }) },
  {
    name: "places/[id]/check-in",
    options: () => ({ title: i18n.t("places:checkIn.title"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
