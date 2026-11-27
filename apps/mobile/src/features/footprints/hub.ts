import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Footprints: how much of the map the account has uncovered, its levels and the ranking. */
export default [
  { id: "footprints", order: 44, href: "/footprints", label: () => i18n.t("footprints:title") },
] satisfies HubEntry[];
