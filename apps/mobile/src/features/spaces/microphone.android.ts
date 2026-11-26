import { PermissionsAndroid } from "react-native";
import i18n from "#/lib/i18n";

/**
 * Android asks for the microphone at run time, before a room opens it. Asked
 * again after a "no", the system lets the app say why first.
 */
export async function microphoneAllowed(): Promise<boolean> {
  const permission = PermissionsAndroid.PERMISSIONS.RECORD_AUDIO;
  if (await PermissionsAndroid.check(permission)) return true;
  const answer = await PermissionsAndroid.request(permission, {
    title: i18n.t("mobile:spaces.microphoneAsk.title"),
    message: i18n.t("mobile:spaces.microphoneAsk.body"),
    buttonPositive: i18n.t("common:actions.ok"),
  });
  return answer === PermissionsAndroid.RESULTS.GRANTED;
}
