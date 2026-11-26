import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "settings/index", options: () => ({ title: i18n.t("mobile:settings.title") }) },
  {
    name: "settings/profile",
    options: () => ({ title: i18n.t("settings:sections.profile.title") }),
  },
  {
    name: "settings/privacy",
    options: () => ({ title: i18n.t("settings:sections.privacy.title") }),
  },
  {
    name: "settings/security",
    options: () => ({ title: i18n.t("settings:sections.security.title") }),
  },
  {
    name: "settings/totp",
    options: () => ({
      title: i18n.t("settings:security.totp.setupTitle"),
      presentation: "modal",
    }),
  },
  { name: "licenses", options: () => ({ title: i18n.t("mobile:licenses.title") }) },
] satisfies ScreenEntry[];
