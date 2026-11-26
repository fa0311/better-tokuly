import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "emoji/index", options: () => ({ title: i18n.t("emoji:title") }) },
  { name: "emoji/[name]", options: () => ({ title: i18n.t("emoji:title") }) },
] satisfies ScreenEntry[];
