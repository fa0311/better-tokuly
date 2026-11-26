import { MessageCircle } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  {
    name: "chats",
    order: 20,
    title: () => i18n.t("mobile:tabs.chat"),
    icon: MessageCircle,
    badge: "chat",
  },
] satisfies TabEntry[];
