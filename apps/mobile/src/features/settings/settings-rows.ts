import { router } from "expo-router";
import { Scale, ShieldHalf } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";

export default [
  {
    id: "privacy",
    order: 4,
    icon: ShieldHalf,
    label: () => i18n.t("settings:sections.privacy.title"),
    hint: () => i18n.t("settings:sections.privacy.hint"),
    onPress: () => router.push("/settings/privacy"),
  },
  {
    id: "licenses",
    order: 30,
    icon: Scale,
    label: () => i18n.t("mobile:settings.licenses"),
    onPress: () => router.push("/licenses"),
  },
] satisfies SettingsRow[];
