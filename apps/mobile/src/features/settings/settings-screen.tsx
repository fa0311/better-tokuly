import { LogOut } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Page, Row, Section } from "#/components/page";
import { signOut } from "#/features/session/session";
import { SERVER_URL } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { appVersion } from "#/platform/device";
import { SETTINGS_CARDS, SETTINGS_ROWS } from "#/seams/settings";

/**
 * The native settings so far: the features' rows and cards (`#/seams/settings`: privacy, accounts, push
 * on this device) and signing out; the rest opens on the web.
 */
export function SettingsScreen() {
  const { t } = useTranslation("mobile");
  const { t: tc } = useTranslation("common");
  const { colors } = useTheme();
  // Offline, the other devices are signed out later: the person is told, not left to assume.
  const signOutEverywhere = async () => {
    const { everywherePending } = await signOut(true);
    if (everywherePending) Alert.alert(t("settings.signOutAllPending"));
  };
  const confirmSignOut = () =>
    Alert.alert(t("settings.signOutConfirm"), undefined, [
      { text: tc("actions.cancel"), style: "cancel" },
      {
        text: t("settings.signOutAll"),
        style: "destructive",
        onPress: () => void signOutEverywhere(),
      },
      { text: t("settings.signOut"), style: "destructive", onPress: () => void signOut() },
    ]);
  return (
    <Page testID="settings-screen">
      <Section>
        {SETTINGS_ROWS.map((row) => (
          <Row
            key={row.id}
            testID={`settings-${row.id}`}
            icon={<row.icon size={20} color={colors.ink} />}
            label={row.label()}
            hint={row.hint?.()}
            onPress={row.onPress}
          />
        ))}
      </Section>
      <Text className="px-1 text-xs text-ink-soft">{t("settings.webNote")}</Text>
      {SETTINGS_CARDS.map((card) => (
        <card.Component key={card.id} />
      ))}
      <Section>
        <Row
          testID="settings-sign-out"
          icon={<LogOut size={20} color={colors.danger} />}
          label={t("settings.signOut")}
          danger
          onPress={confirmSignOut}
        />
      </Section>
      <View className="items-center gap-1 pt-4">
        <Text className="text-xs text-ink-faint">
          {t("settings.version", { version: appVersion })}
        </Text>
        <Text className="text-xs text-ink-faint">{t("settings.server", { url: SERVER_URL })}</Text>
      </View>
    </Page>
  );
}
