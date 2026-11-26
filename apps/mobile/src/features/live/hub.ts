import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Live streams: friends on air, the channels the viewer follows, everyone public. */
export default [
  { id: "live", order: 30, href: "/live", label: () => i18n.t("live:tiles.watch") },
] satisfies HubEntry[];
