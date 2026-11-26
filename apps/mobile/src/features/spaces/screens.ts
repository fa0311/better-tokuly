import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "spaces/index", options: () => ({ title: i18n.t("spaces:title") }) },
  { name: "spaces/[id]", options: () => ({ title: "" }) },
  { name: "spaces/hangouts/[id]", options: () => ({ title: "" }) },
] satisfies ScreenEntry[];
