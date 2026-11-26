import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "plans/index", options: () => ({ title: i18n.t("plans:plan.title") }) },
  {
    name: "plans/new",
    options: () => ({ title: i18n.t("plans:plan.newTitle"), presentation: "modal" }),
  },
  { name: "plans/[id]/index", options: () => ({ title: "" }) },
  {
    name: "plans/[id]/edit",
    options: () => ({ title: i18n.t("plans:plan.editTitle"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
