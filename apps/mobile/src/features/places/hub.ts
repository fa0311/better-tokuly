import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Places: the own ones, the saved ones, finding more. */
export default [
  { id: "places", order: 43, href: "/places", label: () => i18n.t("places:title") },
] satisfies HubEntry[];
