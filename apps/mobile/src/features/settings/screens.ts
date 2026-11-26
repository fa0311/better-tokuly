import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "settings/index", options: () => ({ title: i18n.t("mobile:settings.title") }) },
  {
    name: "settings/privacy",
    options: () => ({ title: i18n.t("settings:sections.privacy.title") }),
  },
  { name: "licenses", options: () => ({ title: i18n.t("mobile:licenses.title") }) },
] satisfies ScreenEntry[];
