import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

export default [
  { id: "moments", order: 12, href: "/moments", label: () => i18n.t("moments:hub") },
] satisfies HubEntry[];
