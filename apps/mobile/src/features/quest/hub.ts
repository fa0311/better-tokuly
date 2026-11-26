import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Quests: the level, today's missions and the badges, with the rewards waiting to be collected. */
export default [
  { id: "quest", order: 27, href: "/quest", label: () => i18n.t("quest:title"), badge: "quest" },
] satisfies HubEntry[];
