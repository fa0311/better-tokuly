import { Clapperboard } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  {
    name: "shorts",
    order: 30,
    title: () => i18n.t("mobile:tabs.shorts"),
    icon: Clapperboard,
    noHeader: true,
  },
] satisfies TabEntry[];
