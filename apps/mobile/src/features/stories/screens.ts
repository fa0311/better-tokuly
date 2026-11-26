import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  {
    name: "stories/[userId]",
    options: () => ({ headerShown: false, presentation: "fullScreenModal", animation: "fade" }),
  },
  {
    name: "stories/new",
    options: () => ({ title: i18n.t("stories:create.title"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
