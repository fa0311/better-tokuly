import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "live/index", options: () => ({ title: i18n.t("live:browse.title") }) },
  // The watch screen draws its own back button over the picture.
  { name: "live/[id]", options: () => ({ headerShown: false }) },
] satisfies ScreenEntry[];
