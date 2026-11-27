import i18n from "#/lib/i18n";
import type { HubEntry } from "#/seams/hub";

/** 絵文字: the server's custom emoji and the person's deck. */
export default [
  { id: "emoji", order: 46, href: "/emoji", label: () => i18n.t("emoji:title") },
] satisfies HubEntry[];
