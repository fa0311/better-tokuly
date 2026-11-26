import { router } from "expo-router";
import { Bell } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";

export default [
  {
    id: "notifications",
    order: 5,
    icon: Bell,
    label: () => i18n.t("notifications:settings.title"),
    hint: () => i18n.t("settings:sections.notifications.hint"),
    onPress: () => router.push("/settings/notifications"),
  },
] satisfies SettingsRow[];
