import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "more", options: () => ({ title: i18n.t("mobile:more.title") }) },
  // The page's own title replaces it once it has loaded.
  { name: "web", options: () => ({ title: "" }) },
] satisfies ScreenEntry[];
