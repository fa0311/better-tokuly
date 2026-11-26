import { router } from "expo-router";
import { ExternalLink, Scale } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";
import { openWebSettings } from "./web-settings";

export default [
  {
    id: "web",
    order: 20,
    icon: ExternalLink,
    label: () => i18n.t("mobile:settings.web"),
    hint: () => i18n.t("mobile:settings.webHint"),
    onPress: () => void openWebSettings(),
  },
  {
    id: "licenses",
    order: 30,
    icon: Scale,
    label: () => i18n.t("mobile:settings.licenses"),
    onPress: () => router.push("/licenses"),
  },
] satisfies SettingsRow[];
