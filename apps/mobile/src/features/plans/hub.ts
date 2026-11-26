import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Plans: the viewer's and their friends' plans, invitations waiting for an answer. */
export default [
  {
    id: "plans",
    order: 25,
    href: "/plans",
    label: () => i18n.t("plans:plan.title"),
    badge: "plans",
  },
] satisfies HubEntry[];
