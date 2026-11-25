import { useTranslation } from "react-i18next";
import { Linking, Text, View } from "react-native";
import { Button } from "#/components/button";
import { disablePush, enablePush, usePush, usePushStatus } from "./push";

/**
 * Push on this device for the signed-in account: what it does, and the one
 * button that changes it (or the way to the settings app when the OS says no).
 */
export function PushCard({ compact }: { compact?: boolean }) {
  const { t } = useTranslation("mobile");
  const status = usePushStatus();
  const failed = usePush((state) => state.failed);
  if (status === "loading" || (compact && (status === "on" || status === "unavailable")))
    return null;
  return (
    <View testID="push-card" className="gap-2 rounded-card bg-surface p-4">
      <Text className="font-black text-ink">{t("push.title")}</Text>
      <Text className="text-sm text-ink-soft">{t(`push.lead.${status}`)}</Text>
      {failed && status === "on" && (
        <Text accessibilityRole="alert" className="text-sm font-bold text-danger">
          {t("push.failed")}
        </Text>
      )}
      {status === "off" && (
        <Button testID="push-enable" size="sm" onPress={() => void enablePush()}>
          {t("push.enable")}
        </Button>
      )}
      {status === "on" && (
        <Button
          testID="push-disable"
          size="sm"
          variant="secondary"
          onPress={() => void disablePush()}
        >
          {t("push.disable")}
        </Button>
      )}
      {status === "denied" && (
        <Button size="sm" variant="secondary" onPress={() => void Linking.openSettings()}>
          {t("sharing.openSettings")}
        </Button>
      )}
    </View>
  );
}
