import { Smile } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

/** The own profile, with the hub's entries under it. */
export default [
  { name: "me", order: 60, title: () => i18n.t("mobile:tabs.me"), icon: Smile },
] satisfies TabEntry[];
