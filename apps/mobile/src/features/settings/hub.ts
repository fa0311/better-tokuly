import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

export default [
  { id: "settings", order: 90, href: "/settings", label: () => i18n.t("mobile:settings.title") },
] satisfies HubEntry[];
