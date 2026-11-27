import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** The history: the calendar of days, visits, nights and the numbers. */
export default [
  { id: "history", order: 44, href: "/history", label: () => i18n.t("history:title") },
] satisfies HubEntry[];
