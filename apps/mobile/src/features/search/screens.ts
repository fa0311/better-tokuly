import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "search", options: () => ({ title: i18n.t("search:title") }) },
] satisfies ScreenEntry[];
