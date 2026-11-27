import { useEffect, useState } from "react";
import { Alert, AppState, Linking } from "react-native";
import i18n from "#/lib/i18n";
import {
  askPushPermission,
  type PushPermission,
  readPushPermission,
} from "#/platform/notifications";
import { readSecure, writeSecure } from "#/platform/secure-storage";

/**
 * Whether the timer's reminder (`session.ts`, `remindBeforeIdle`) can reach
 * the person: it is a local notification, so it needs the OS's permission.
 * Allowing it here registers nothing for push: that stays the account's own
 * choice in the notification settings.
 */

/** Set once the person was asked on this device: "later" is an answer too. */
export const ASKED_KEY = "school.reminders.asked";

/**
 * Before the first start on this device: says why, then shows the OS
 * prompt. Asks once, and only while the OS can still ask; resolves with the
 * answer and never rejects (declining, or failing to ask, still starts the timer).
 */
export async function askForReminders(idleSeconds: number): Promise<PushPermission> {
  const permission = await readPushPermission().catch((): PushPermission => "denied");
  if (permission !== "undetermined") return permission;
  try {
    if (await readSecure(ASKED_KEY)) return permission;
    await writeSecure(ASKED_KEY, "1");
  } catch {
    // A phone that cannot note the answer would be asked at every start: the timer starts without asking.
    return permission;
  }
  return new Promise((resolve) => {
    Alert.alert(
      i18n.t("mobile:school.reminderAsk.title"),
      i18n.t("mobile:school.reminderAsk.body", { minutes: Math.round(idleSeconds / 60) }),
      [
        {
          text: i18n.t("mobile:school.reminderAsk.later"),
          style: "cancel",
          onPress: () => resolve("undetermined"),
        },
        {
          text: i18n.t("mobile:school.reminderAsk.allow"),
          onPress: () => {
            askPushPermission().then(resolve, () => resolve("denied"));
          },
        },
      ],
      { cancelable: false },
    );
  });
}

/** The permission as it is now: read again whenever the app comes back (it may have been changed in the settings). */
export function useReminderPermission() {
  const [permission, setPermission] = useState<PushPermission | undefined>(undefined);
  useEffect(() => {
    const read = () => void readPushPermission().then(setPermission, () => setPermission("denied"));
    read();
    const appState = AppState.addEventListener("change", (state) => {
      if (state === "active") read();
    });
    return () => appState.remove();
  }, []);
  /** The prompt while the OS can still show it, the system settings after that. */
  const turnOn = async () => {
    if (permission === "undetermined") setPermission(await askPushPermission());
    else await Linking.openSettings();
  };
  return { permission, turnOn };
}
