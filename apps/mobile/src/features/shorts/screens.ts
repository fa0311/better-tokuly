import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  {
    name: "shorts/watch",
    options: () => ({
      title: "",
      headerTransparent: true,
      headerTintColor: "#ffffff",
      contentStyle: { backgroundColor: "#000000" },
    }),
  },
  { name: "shorts/tag/[tag]", options: () => ({ title: "" }) },
  { name: "shorts/sound/[id]", options: () => ({ title: i18n.t("shorts:sound.pageTitle") }) },
  {
    name: "shorts/new",
    options: () => ({ title: i18n.t("shorts:create.title"), presentation: "modal" }),
  },
  { name: "shorts/mine", options: () => ({ title: i18n.t("shorts:studio.title") }) },
] satisfies ScreenEntry[];
