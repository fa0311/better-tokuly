import { PermissionsAndroid, Platform } from "react-native";
import i18n from "#/lib/i18n";

/** Asked at most once while the app runs: a "no" keeps the phone's own speaker and earpiece. */
let asked = false;

/**
 * Android 12 and later let the sound go to a Bluetooth headset only with
 * BLUETOOTH_CONNECT, asked when a room or hangout opens. Older versions
 * grant Bluetooth with the app. The answer only changes where the sound
 * goes: entering never waits on a "yes".
 */
export async function askForBluetooth(): Promise<void> {
  if (asked || (Platform.Version as number) < 31) return;
  asked = true;
  const permission = PermissionsAndroid.PERMISSIONS.BLUETOOTH_CONNECT;
  if (await PermissionsAndroid.check(permission)) return;
  await PermissionsAndroid.request(permission, {
    title: i18n.t("mobile:spaces.bluetoothAsk.title"),
    message: i18n.t("mobile:spaces.bluetoothAsk.body"),
    buttonPositive: i18n.t("common:actions.ok"),
  });
}
