import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "plans/index", options: () => ({ title: i18n.t("plans:plan.title") }) },
  { name: "plans/[id]/index", options: () => ({ title: "" }) },
] satisfies ScreenEntry[];
