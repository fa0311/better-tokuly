import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** School: today's classes, what is due, friends who are studying. */
export default [
  { id: "school", order: 40, href: "/school", label: () => i18n.t("school:title") },
] satisfies HubEntry[];
