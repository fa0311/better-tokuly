import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Text, View } from "react-native";
import { Button } from "#/components/button";
import {
  askAlways,
  confirmStop,
  stopSharing,
  turnOn,
  useSharing,
  useSharingState,
} from "./sharing";

/**
 * A stop is owed from the moment it is sent: the card says it has not
 * reached the server only once it is still owed this long after.
 */
export const STOP_NOTICE_MS = 4_000;

/** `owed`, once it stayed true for `STOP_NOTICE_MS`. */
function useStillOwed(owed: boolean): boolean {
  const [still, setStill] = useState(false);
  useEffect(() => {
    setStill(false);
    if (!owed) return;
    const timer = setTimeout(() => setStill(true), STOP_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [owed]);
  return owed && still;
}

/**
 * Where this device stands with sharing, and the next step: the consent
 * (with what the OS will ask and why), "always" for while the app is closed,
 * the settings app when the permission was refused, and stopping.
 */
export function SharingCard() {
  const { t } = useTranslation("mobile");
  const { t: tm } = useTranslation("map");
  const state = useSharingState();
  const access = useSharing((s) => s.access);
  const stopOwed = useStillOwed(useSharing((s) => s.stopOwed));

  if (state === "loading") return null;
  return (
    <View className="gap-3" testID={`sharing-${state}`}>
      {stopOwed && (
        <View className="gap-2 rounded-control bg-danger/10 p-3">
          <Text className="text-sm font-black text-danger">{tm("sharing.stopFailed.title")}</Text>
          <Text className="text-xs text-ink-soft">{tm("sharing.stopFailed.body")}</Text>
          <Button size="sm" variant="secondary" onPress={confirmStop}>
            {tm("sharing.tellAgain")}
          </Button>
        </View>
      )}
      {state === "needsConsent" && (
        <>
          <Title title={t("sharing.explain.title")} body={t("sharing.explain.body")} />
          <Text className="text-xs font-bold text-ink-soft">
            ① {t("sharing.explain.whileUsing")}
          </Text>
          <Text className="text-xs font-bold text-ink-soft">② {t("sharing.explain.always")}</Text>
          <Button testID="sharing-turn-on" block onPress={turnOn}>
            {t("sharing.turnOn")}
          </Button>
        </>
      )}
      {state === "blocked" && (
        <>
          <Title title={t("sharing.blocked.title")} body={t("sharing.blocked.body")} />
          <Button block onPress={() => void Linking.openSettings()}>
            {t("sharing.openSettings")}
          </Button>
        </>
      )}
      {state === "servicesOff" && (
        <Title title={t("sharing.servicesOff.title")} body={t("sharing.servicesOff.body")} />
      )}
      {state === "stopped" && (
        <>
          <Title title={t("sharing.stopped.title")} body={t("sharing.stopped.body")} />
          <Button testID="sharing-resume" block onPress={turnOn}>
            {t("sharing.resume")}
          </Button>
        </>
      )}
      {(state === "sharing" || state === "starting") && (
        <>
          <Title
            title={state === "sharing" ? t("sharing.sharing.title") : tm("sharing.starting.title")}
            body={
              access?.background ? t("sharing.sharing.background") : t("sharing.sharing.foreground")
            }
          />
          {access?.approximate && (
            <Text className="rounded-control bg-sun/30 px-3 py-2 text-xs font-bold text-ink">
              {t("sharing.approximate")}
            </Text>
          )}
          {!access?.background && (
            <View className="gap-1">
              <Button testID="sharing-always" variant="lime" block onPress={askAlways}>
                {t("sharing.alwaysAsk")}
              </Button>
              <Text className="text-center text-[11px] font-bold text-ink-faint">
                {t("sharing.alwaysHint")}
              </Text>
            </View>
          )}
          <Button testID="sharing-stop" variant="ghost" block onPress={stopSharing}>
            {t("sharing.stop")}
          </Button>
        </>
      )}
    </View>
  );
}

function Title({ title, body }: { title: string; body: string }) {
  return (
    <View className="gap-1">
      <Text className="text-lg font-black text-ink">{title}</Text>
      <Text className="text-sm text-ink-soft">{body}</Text>
    </View>
  );
}
