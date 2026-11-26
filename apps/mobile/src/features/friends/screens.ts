import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "friends", options: () => ({ title: i18n.t("mobile:tabs.friends") }) },
  {
    name: "add-friend",
    options: () => ({ title: i18n.t("mobile:friends.addTitle"), presentation: "modal" }),
  },
  { name: "invite/[code]", options: () => ({ title: "" }) },
] satisfies ScreenEntry[];
