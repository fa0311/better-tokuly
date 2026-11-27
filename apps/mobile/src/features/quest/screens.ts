import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "quest/index", options: () => ({ title: i18n.t("quest:title") }) },
] satisfies ScreenEntry[];
