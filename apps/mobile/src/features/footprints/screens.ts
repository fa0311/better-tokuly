import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "footprints", options: () => ({ title: i18n.t("footprints:title") }) },
] satisfies ScreenEntry[];
