import { common, create, identity } from "@tokuly/api";
import * as Application from "expo-application";
import { randomUUID } from "expo-crypto";
import * as Device from "expo-device";
import { getCalendars, getLocales } from "expo-localization";
import { Platform } from "react-native";
import { readSecure, writeSecure } from "./secure-storage";

const DEVICE_ID_KEY = "tokuly.deviceId";
let deviceIdValue: Promise<string> | null = null;

/**
 * A random ID for this install, kept in secure storage: the server
 * de-duplicates sessions and counts the accounts of a device by it.
 */
export function deviceId(): Promise<string> {
  deviceIdValue ??= (async () => {
    const stored = await readSecure(DEVICE_ID_KEY);
    if (stored) return stored;
    const id = randomUUID();
    await writeSecure(DEVICE_ID_KEY, id);
    return id;
  })();
  return deviceIdValue;
}

export const platform: common.Platform =
  Platform.OS === "ios" ? common.Platform.IOS : common.Platform.ANDROID;

export const apiClientKind = Platform.OS === "ios" ? "ios" : "android";

export const appVersion = Application.nativeApplicationVersion ?? "dev";

/** What a sign-in tells the server about this device (shown in the account's device list). */
export async function clientInfo(): Promise<identity.ClientInfo> {
  return create(identity.ClientInfoSchema, {
    platform,
    deviceName: Device.deviceName ?? Device.modelName ?? Platform.OS,
    appVersion,
    deviceId: await deviceId(),
    locale: getLocales()[0]?.languageTag ?? "ja-JP",
    timeZone: getCalendars()[0]?.timeZone ?? "Asia/Tokyo",
  });
}
