import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** The wallet: the coins, today's login bonus and the history; its count is a bonus waiting to be claimed. */
export default [
  {
    id: "wallet",
    order: 28,
    href: "/wallet",
    label: () => i18n.t("wallet:title"),
    badge: "wallet",
  },
] satisfies HubEntry[];
