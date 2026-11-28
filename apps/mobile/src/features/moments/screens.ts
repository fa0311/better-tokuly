import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "moments/index", options: () => ({ title: i18n.t("moments:title") }) },
  { name: "moments/friends/index", options: () => ({ title: i18n.t("moments:friends.title") }) },
  {
    name: "moments/friends/[momentId]",
    options: () => ({ title: i18n.t("moments:friends.detail") }),
  },
] satisfies ScreenEntry[];
