import * as Notifications from "expo-notifications";
import { Platform } from "react-native";

/**
 * The device's side of notifications (`expo-notifications`, the OS's own
 * push services: FCM on Android, APNs on iOS — no Expo push service): the
 * permission, the device token, Android's channels, the icon badge, and
 * what a tap on a notification carries.
 */

export type PushPermission = "granted" | "denied" | "undetermined";

function permissionOf(status: Notifications.NotificationPermissionsStatus): PushPermission {
  if (status.granted) return "granted";
  // iOS: provisional ("delivered quietly") counts as allowed.
  if (status.ios?.status === Notifications.IosAuthorizationStatus.PROVISIONAL) return "granted";
  return status.canAskAgain ? "undetermined" : "denied";
}

export async function readPushPermission(): Promise<PushPermission> {
  return permissionOf(await Notifications.getPermissionsAsync());
}

/** Asks the OS (Android 13+, iOS); answers at once when it was decided before. */
export async function askPushPermission(): Promise<PushPermission> {
  return permissionOf(
    await Notifications.requestPermissionsAsync({
      ios: { allowAlert: true, allowBadge: true, allowSound: true },
    }),
  );
}

/** The token the OS push service knows this install by. Throws when the build has no push set up. */
export async function devicePushToken(): Promise<{ platform: "ios" | "android"; token: string }> {
  const { type, data } = await Notifications.getDevicePushTokenAsync();
  if (type !== "ios" && type !== "android") throw new Error(`no native push on ${type}`);
  return { platform: type, token: String(data) };
}

/** Called when the OS hands out a new token (it may rotate one at any time). */
export function onPushTokenChange(listener: () => void): () => void {
  const subscription = Notifications.addPushTokenListener(() => listener());
  return () => subscription.remove();
}

/**
 * Throws the device token away: the push service stops delivering to it,
 * whoever registered it. The last resort when the server could not be told
 * that an account left this device.
 */
export async function dropDevicePushToken(): Promise<void> {
  await Notifications.unregisterForNotificationsAsync();
}

/** Takes the notifications of an account that left off the screen, and its count off the icon. */
export async function clearPresented(): Promise<void> {
  await Notifications.dismissAllNotificationsAsync();
  await Notifications.setBadgeCountAsync(0);
}

export async function setIconBadge(count: number): Promise<void> {
  await Notifications.setBadgeCountAsync(Math.max(0, count));
}

/**
 * Android shows a notification in the channel the server names: one per
 * notification category. Channels keep the device's lock-screen rule
 * (PRIVATE: content hidden on a secure lock screen when the person chose
 * that in the system settings); how much the payload carries at all is the
 * account's preview setting, applied by the server.
 */
export async function prepareChannels(names: Record<string, string>): Promise<void> {
  if (Platform.OS !== "android") return;
  await Promise.all(
    Object.entries(names).map(([id, name]) =>
      Notifications.setNotificationChannelAsync(id, {
        name,
        importance:
          id === "chat"
            ? Notifications.AndroidImportance.HIGH
            : Notifications.AndroidImportance.DEFAULT,
        lockscreenVisibility: Notifications.AndroidNotificationVisibility.PRIVATE,
        showBadge: true,
      }),
    ),
  );
}

/** What the server put beside a notification to route a tap (`path`, `notification_id`). */
export interface PushTap {
  key: string;
  path: string;
  notificationId: string;
}

function tapOf(response: Notifications.NotificationResponse): PushTap {
  const content = response.notification.request.content;
  // FCM puts the data block in `data`; APNs keeps the routing keys at the top of the payload, which iOS also hands over as `data`.
  const data = (content.data ?? {}) as Record<string, unknown>;
  return {
    key: response.notification.request.identifier,
    path: typeof data.path === "string" ? data.path : "",
    notificationId: typeof data.notification_id === "string" ? data.notification_id : "",
  };
}

/** Taps on notifications: the one that opened the app (if any), then each new one. */
export function onPushTap(listener: (tap: PushTap) => void): () => void {
  const opened = Notifications.getLastNotificationResponse();
  if (opened) listener(tapOf(opened));
  const subscription = Notifications.addNotificationResponseReceivedListener((response) =>
    listener(tapOf(response)),
  );
  return () => subscription.remove();
}

/** Forgets the tap that opened the app, so a later start (another account) does not follow it again. */
export function forgetOpeningTap(): void {
  Notifications.clearLastNotificationResponse();
}

/**
 * How a notification that arrives while the app is open is shown: as a
 * banner, unless `quiet` says the screen already shows it (the conversation
 * it is about is open).
 */
export function showWhileOpen(quiet: (path: string) => boolean): void {
  Notifications.setNotificationHandler({
    handleNotification: async (notification) => {
      const data = (notification.request.content.data ?? {}) as Record<string, unknown>;
      const show = !quiet(typeof data.path === "string" ? data.path : "");
      return {
        shouldShowBanner: show,
        shouldShowList: show,
        shouldPlaySound: show,
        shouldSetBadge: true,
      };
    },
  });
}
