import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "accounts", options: () => ({ title: i18n.t("accounts:title") }) },
  {
    name: "add-account",
    options: () => ({ title: i18n.t("accounts:addTitle"), presentation: "modal" }),
  },
  {
    name: "create-account",
    options: () => ({ title: i18n.t("accounts:create"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
