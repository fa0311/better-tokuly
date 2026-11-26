import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** スタンプ: the store, and the packs in the picker. */
export default [
  { id: "stamps", order: 45, href: "/stamps", label: () => i18n.t("stamps:title") },
] satisfies HubEntry[];
