import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "history/index", options: () => ({ title: i18n.t("history:title") }) },
  { name: "history/[date]", options: () => ({ title: i18n.t("history:title") }) },
  {
    name: "history/delete",
    options: () => ({ title: i18n.t("history:delete.title"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
