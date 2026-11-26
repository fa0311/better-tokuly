import { Users } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  {
    name: "friends",
    order: 40,
    title: () => i18n.t("mobile:tabs.friends"),
    icon: Users,
    badge: "friends",
  },
] satisfies TabEntry[];
