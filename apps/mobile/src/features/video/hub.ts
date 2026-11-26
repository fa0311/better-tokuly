import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** Long videos have no tab: the feeds open from the Me tab. */
export default [
  { id: "video", order: 20, href: "/watch", label: () => i18n.t("video:app.hub") },
] satisfies HubEntry[];
