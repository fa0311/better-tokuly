import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Communities: the caller's own, finding more, and what waits in them. */
export default [
  {
    id: "community",
    order: 25,
    href: "/communities",
    label: () => i18n.t("community:title"),
    badge: "community",
  },
] satisfies HubEntry[];
