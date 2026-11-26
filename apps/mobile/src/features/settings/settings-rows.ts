import { router } from "expo-router";
import { LockKeyhole, Scale, ShieldHalf, UserRound } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";

export default [
  {
    id: "profile",
    order: 2,
    icon: UserRound,
    label: () => i18n.t("settings:sections.profile.title"),
    hint: () => i18n.t("settings:sections.profile.hint"),
    onPress: () => router.push("/settings/profile"),
  },
  {
    id: "privacy",
    order: 4,
    icon: ShieldHalf,
    label: () => i18n.t("settings:sections.privacy.title"),
    hint: () => i18n.t("settings:sections.privacy.hint"),
    onPress: () => router.push("/settings/privacy"),
  },
  {
    id: "security",
    order: 6,
    icon: LockKeyhole,
    label: () => i18n.t("settings:sections.security.title"),
    hint: () => i18n.t("settings:sections.security.hint"),
    onPress: () => router.push("/settings/security"),
  },
  {
    id: "licenses",
    order: 30,
    icon: Scale,
    label: () => i18n.t("mobile:settings.licenses"),
    onPress: () => router.push("/licenses"),
  },
] satisfies SettingsRow[];
