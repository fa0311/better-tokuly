import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "moments/index", options: () => ({ title: i18n.t("moments:title") }) },
  { name: "moments/calendar", options: () => ({ title: i18n.t("moments:calendar.title") }) },
  { name: "moments/friends/index", options: () => ({ title: i18n.t("moments:today.friends") }) },
  { name: "moments/recap", options: () => ({ title: i18n.t("moments:recap.title") }) },
  {
    name: "moments/friends/[momentId]",
    options: () => ({ title: i18n.t("moments:today.friends") }),
  },
] satisfies ScreenEntry[];
