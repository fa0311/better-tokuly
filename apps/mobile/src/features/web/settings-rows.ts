import { router } from "expo-router";
import { ExternalLink } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { SettingsRow } from "#/seams/settings";
import { webScreenHref } from "./handoff";

/** The settings the app does not have yet: the web's, signed in. */
export default [
  {
    id: "web",
    order: 20,
    icon: ExternalLink,
    label: () => i18n.t("mobile:settings.web"),
    hint: () => i18n.t("mobile:settings.webHint"),
    onPress: () => router.push(webScreenHref("/settings") as never),
  },
] satisfies SettingsRow[];
