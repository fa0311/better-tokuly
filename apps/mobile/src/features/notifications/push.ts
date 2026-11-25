import { common, notify } from "@tokuly/api";
import * as Device from "expo-device";
import { getLocales } from "expo-localization";
import { useEffect } from "react";
import { AppState } from "react-native";
import { create } from "zustand";
import { asParked, onBeforeLeave, useMe, useSessionStore } from "#/features/session/session";
import { api } from "#/lib/api";
import { APNS_SANDBOX } from "#/lib/config";
import { platform } from "#/platform/device";
import {
  askPushPermission,
  clearPresented,
  devicePushToken,
  dropDevicePushToken,
  onPushTokenChange,
  type PushPermission,
  readPushPermission,
} from "#/platform/notifications";
import { readJson, readSecure, writeJson, writeSecure } from "#/platform/secure-storage";

export const pushClient = () => api().service(notify.PushService);

/**
 * What an account decided about push on this device. Like location sharing,
 * the decision belongs to the account: another account signing in here
 * starts undecided, whatever the OS permission says, and this device's token
 * is registered for an account only once it said yes here.
 */
export type PushChoice = "on" | "off" | null;

export const pushChoiceKey = (userId: string) => `tokuly.push.${userId}`;

interface PushStore {
  /** Whose choice this is; `loaded` once it was read. */
  userId: string | null;
  loaded: boolean;
  choice: PushChoice;
  permission: PushPermission | null;
  /** Whether the server delivers to this kind of phone (it has FCM / APNs credentials). */
  available: boolean | null;
  /** The last registration failed (shown with a retry). */
  failed: boolean;
  /** The account is leaving: nothing is registered for it any more. */
  leaving: boolean;
}

export const usePush = create<PushStore>(() => ({
  userId: null,
  loaded: false,
  choice: null,
  permission: null,
  available: null,
  failed: false,
  leaving: false,
}));

/** The push service of this phone: APNs on iOS, FCM on Android. */
const OWN_CHANNEL =
  platform === common.Platform.IOS ? notify.PushChannelKind.APNS : notify.PushChannelKind.FCM;

/** Whether the server delivers through this phone's push service. */
async function readAvailability(): Promise<boolean> {
  try {
    const { channels } = await pushClient().getPushConfig({});
    return channels.includes(OWN_CHANNEL);
  } catch {
    return false;
  }
}

function stillFor(userId: string): boolean {
  const state = usePush.getState();
  return state.userId === userId && state.choice === "on" && !state.leaving;
}

/** The registration on its way, if any: a removal waits for it, or it could land after the removal. */
let registering: Promise<void> | null = null;

/** Registers this phone's token for the signed-in session (again: a token may have rotated). */
function register(userId: string): Promise<void> {
  const run = sendRegistration(userId).finally(() => {
    if (registering === run) registering = null;
  });
  registering = run;
  return run;
}

async function sendRegistration(userId: string): Promise<void> {
  try {
    const { platform: kind, token } = await devicePushToken();
    if (!stillFor(userId)) return;
    await pushClient().registerDevice({
      token:
        kind === "ios"
          ? { case: "apns", value: { token, sandbox: APNS_SANDBOX } }
          : { case: "fcmToken", value: token },
      platform,
      locale: getLocales()[0]?.languageTag ?? "",
      deviceName: Device.deviceName ?? Device.modelName ?? "",
    });
    if (usePush.getState().userId === userId) usePush.setState({ failed: false });
  } catch {
    if (usePush.getState().userId === userId) usePush.setState({ failed: true });
  }
}

async function setChoice(userId: string, choice: PushChoice): Promise<void> {
  if (usePush.getState().userId === userId) usePush.setState({ choice });
  await writeSecure(pushChoiceKey(userId), choice);
}

/**
 * The signed-in account starts on this device: its own choice, the OS
 * permission and the server's channels are read, and when all three say yes
 * the token is registered for this session.
 */
export async function startPush(userId: string, sessionId: string): Promise<void> {
  usePush.setState({
    userId,
    loaded: false,
    choice: null,
    permission: null,
    available: null,
    failed: false,
    leaving: false,
  });
  await settleOwed(sessionId);
  const [stored, permission, available] = await Promise.all([
    readSecure(pushChoiceKey(userId)),
    readPushPermission().catch((): PushPermission => "denied"),
    readAvailability(),
  ]);
  if (usePush.getState().userId !== userId || usePush.getState().leaving) return;
  const choice: PushChoice = stored === "on" || stored === "off" ? stored : null;
  usePush.setState({ loaded: true, choice, permission, available });
  if (choice === "on" && permission === "granted" && available) await register(userId);
}

/** The account says yes: the OS asks for the permission (if it has not yet), then the token is registered. */
export async function enablePush(): Promise<void> {
  const { userId } = usePush.getState();
  if (!userId) return;
  const permission = await askPushPermission();
  usePush.setState({ permission });
  if (permission !== "granted" || !usePush.getState().available) return;
  await setChoice(userId, "on");
  await register(userId);
}

/** No push for this account on this device any more. */
export async function disablePush(): Promise<void> {
  const { userId } = usePush.getState();
  if (!userId) return;
  await setChoice(userId, "off");
  await unregister();
}

/**
 * Takes this session's registrations off the server. Resolves to false when
 * the server could not be told: the token itself is thrown away then
 * (Android hands out a new one; iOS may keep the same, so the removal is
 * also owed, see `UNREGISTER_OWED_KEY`).
 */
async function unregister(): Promise<boolean> {
  // The session's own client, taken now: after a wait another account may be the active one.
  const client = pushClient();
  await registering;
  try {
    await client.unregisterDevice({ deviceId: "" });
    return true;
  } catch {
    await dropDevicePushToken().catch(() => {});
    return false;
  }
}

/**
 * Sessions that left this device while their registration could not be
 * removed (offline at a switch). Each start tries again as that session,
 * parked here, until the server has it: like a stop of location sharing that
 * did not reach the server.
 */
export const UNREGISTER_OWED_KEY = "tokuly.push.unregisterOwed";

async function owed(): Promise<string[]> {
  return readJson<string[]>(UNREGISTER_OWED_KEY, []);
}

async function setOwed(sessionId: string, isOwed: boolean): Promise<void> {
  const rest = (await owed()).filter((id) => id !== sessionId);
  await writeJson(UNREGISTER_OWED_KEY, isOwed ? [...rest, sessionId] : rest);
}

/** Removes the registrations still owed: as the active session, or as a parked one. */
async function settleOwed(activeSessionId: string): Promise<void> {
  for (const sessionId of await owed()) {
    try {
      if (sessionId === activeSessionId) await pushClient().unregisterDevice({ deviceId: "" });
      // `null`: the device holds that account no more (signed out), and nothing can be done as it.
      else
        await asParked(sessionId, (account) =>
          account.service(notify.PushService).unregisterDevice({ deviceId: "" }),
        );
      await setOwed(sessionId, false);
    } catch {
      // Still offline, or the session cannot be resumed yet: the next start tries again.
    }
  }
}

/**
 * Before the signed-in account leaves this device (sign-out, another account taking
 * over), while its session still works: its registration goes, so none of
 * its pushes reach the phone in the next account's hands, and what it had on
 * the screen is cleared.
 */
export async function leavePush(sessionId: string): Promise<void> {
  usePush.setState({ leaving: true });
  // Whatever this launch knows about the account's choice (it may still be
  // reading it), a registration from an earlier launch can be alive: the
  // removal always goes out, and the server takes it as done when there is none.
  // Owed until done: the leave hook may give up waiting (a registration hanging on the OS
  // for its token) while this goes on, and the account must not be left registered then.
  const removal = unregister();
  await setOwed(sessionId, true);
  if (await removal) await setOwed(sessionId, false);
  await clearPresented().catch(() => {});
}

/**
 * Runs while someone is signed in: registers the device for the account
 * that said yes, follows token changes and the permission, and unregisters
 * before the account leaves.
 */
export function usePushRegistration(): void {
  const myId = useMe()?.user?.id ?? null;
  const sessionId = useSessionStore((s) =>
    s.state.status === "signedIn" ? s.state.sessionId : null,
  );
  useEffect(() => {
    if (!myId || !sessionId) return;
    void startPush(myId, sessionId);
    const offLeave = onBeforeLeave(() => leavePush(sessionId));
    const offToken = onPushTokenChange(() => {
      if (stillFor(myId)) void register(myId);
    });
    // Back from the settings app: the permission may have changed.
    const appState = AppState.addEventListener("change", (next) => {
      if (next !== "active" || usePush.getState().userId !== myId) return;
      void readPushPermission().then((permission) => {
        const before = usePush.getState().permission;
        usePush.setState({ permission });
        if (before !== "granted" && permission === "granted" && stillFor(myId)) void register(myId);
      });
    });
    return () => {
      offLeave();
      offToken();
      appState.remove();
    };
  }, [myId, sessionId]);
}

/** What the push row in the inbox and the settings shows. */
export type PushStatus = "loading" | "on" | "off" | "denied" | "unavailable";

export function usePushStatus(): PushStatus {
  const { loaded, choice, permission, available } = usePush();
  if (!loaded || permission === null || available === null) return "loading";
  if (!available) return "unavailable";
  if (permission === "denied") return "denied";
  return choice === "on" && permission === "granted" ? "on" : "off";
}
