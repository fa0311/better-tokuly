import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "school/index", options: () => ({ title: i18n.t("school:title") }) },
  { name: "school/study", options: () => ({ title: i18n.t("school:timer.title") }) },
  { name: "school/assignments", options: () => ({ title: i18n.t("school:assignments.title") }) },
  {
    name: "school/new-assignment",
    options: () => ({ title: i18n.t("school:assignments.add"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
