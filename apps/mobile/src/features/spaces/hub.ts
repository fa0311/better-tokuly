import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Rooms and hangouts: who is talking now, what is coming, the viewer's hangouts. */
export default [
  {
    id: "spaces",
    order: 35,
    href: "/spaces",
    label: () => i18n.t("spaces:title"),
    badge: "spaces",
  },
] satisfies HubEntry[];
