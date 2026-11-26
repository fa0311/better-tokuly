import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** 「もっと見る」: everything the app shows through the web for now (`features.ts`). */
export default [
  { id: "more", order: 80, href: "/more", label: () => i18n.t("mobile:more.open") },
] satisfies HubEntry[];
