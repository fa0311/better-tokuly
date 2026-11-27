import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "wallet/index", options: () => ({ title: i18n.t("wallet:title") }) },
  { name: "wallet/history", options: () => ({ title: i18n.t("wallet:history.title") }) },
  { name: "wallet/purchases", options: () => ({ title: i18n.t("wallet:purchases.title") }) },
] satisfies ScreenEntry[];
