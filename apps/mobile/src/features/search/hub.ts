import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** さがす: people, notes, tags, shorts and videos, with the person's own recent searches. */
export default [
  { id: "search", order: 15, href: "/search", label: () => i18n.t("search:title") },
] satisfies HubEntry[];
