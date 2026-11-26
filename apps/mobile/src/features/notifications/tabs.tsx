import { Bell } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  {
    name: "inbox",
    order: 50,
    title: () => i18n.t("mobile:tabs.inbox"),
    icon: Bell,
    badge: "notifications",
  },
] satisfies TabEntry[];
