import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "communities", options: () => ({ title: i18n.t("community:title") }) },
  // The community's own name is its title (set by the screen once it is loaded).
  { name: "g/[handle]/index", options: () => ({ title: i18n.t("community:title") }) },
  { name: "g/[handle]/t/[id]", options: () => ({ title: "" }) },
  { name: "g/[handle]/members", options: () => ({ title: i18n.t("community:tabs.members") }) },
  {
    name: "g/[handle]/new",
    options: () => ({ title: i18n.t("community:compose.threadTitle"), presentation: "modal" }),
  },
  { name: "g/join/[code]", options: () => ({ title: i18n.t("community:invite.action") }) },
] satisfies ScreenEntry[];
