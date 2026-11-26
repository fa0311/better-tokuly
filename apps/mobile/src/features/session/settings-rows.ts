import { router } from "expo-router";
import { Users } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";

export default [
  {
    id: "accounts",
    order: 10,
    icon: Users,
    label: () => i18n.t("mobile:settings.accounts"),
    onPress: () => router.push("/accounts"),
  },
] satisfies SettingsRow[];
