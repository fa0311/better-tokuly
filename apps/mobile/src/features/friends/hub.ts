import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/**
 * Friends, their requests and adding more: the first entry under the own
 * profile. Friends are met on the map and in the chats many times a day;
 * their list is not, so it has no tab. A request that waits still shows on
 * the bar (the hub's tab adds up what waits behind its entries).
 */
export default [
  {
    id: "friends",
    order: 10,
    href: "/friends",
    label: () => i18n.t("mobile:tabs.friends"),
    badge: "friends",
  },
] satisfies HubEntry[];
