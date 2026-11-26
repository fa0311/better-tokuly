import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** 質問箱: the viewer's own box, its inbox and the questions they sent. */
export default [
  { id: "ask", order: 40, href: "/ask", label: () => i18n.t("ask:title"), badge: "ask" },
] satisfies HubEntry[];
