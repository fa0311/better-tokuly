import { Smile } from "lucide-react-native";
import i18n from "#/lib/i18n";
import { HUB_BADGE } from "#/seams/hub";
import type { TabEntry } from "#/seams/tabs";

/** The own profile, with the hub's entries under it: the tab counts what waits behind them. */
export default [
  { name: "me", order: 60, title: () => i18n.t("mobile:tabs.me"), icon: Smile, badge: HUB_BADGE },
] satisfies TabEntry[];
