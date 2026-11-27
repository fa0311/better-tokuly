import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { useSettings } from "#/features/settings/data";

/** Whether the account keeps no history: its privacy setting as the server has it. */
export function useRecordingOff(): boolean {
  return useSettings().data?.privacy?.recordHistory === false;
}

/**
 * Said while the account keeps no history: the screens are empty because
 * nothing is recorded, not because nothing happened.
 */
export function RecordingOff() {
  const { t } = useTranslation("mobile");
  if (!useRecordingOff()) return null;
  return (
    <View testID="history-recording-off" className="gap-1 rounded-card bg-surface-2 p-4">
      <Text className="text-sm font-bold text-ink">{t("history.recordingOff")}</Text>
      <Pressable accessibilityRole="link" onPress={() => router.push("/settings/privacy")}>
        <Text className="text-sm font-black text-brand">{t("history.openPrivacy")}</Text>
      </Pressable>
    </View>
  );
}
