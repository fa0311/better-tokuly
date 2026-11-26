import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  {
    name: "settings/notifications",
    options: () => ({ title: i18n.t("notifications:settings.title") }),
  },
] satisfies ScreenEntry[];
