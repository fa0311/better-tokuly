import {
  type LastReport,
  locationSample,
  type OwnPosition,
  reportDecision,
} from "@tokuly/core/map";
import { AppState } from "react-native";
import { api } from "#/lib/api";
import {
  defineBackgroundLocationTask,
  readBattery,
  stopBackgroundUpdates,
} from "#/platform/location";
import { readSecure } from "#/platform/secure-storage";
import { locationClient } from "./data";
import { BACKGROUND_OWNER_KEY, choiceKey } from "./sharing";

let last: LastReport | null = null;
const ANY_PACE = { movingSeconds: 0, stationarySeconds: 0 };

/**
 * Fixes the OS delivers while the app is off screen (or not running at all:
 * then this is the only code that runs). They are sent only for the account
 * that turned sharing on and is still the one signed in here; anything else
 * stops the background updates for good. On screen, the foreground watcher
 * sends instead.
 */
async function reportInBackground(positions: OwnPosition[]): Promise<void> {
  if (AppState.currentState === "active") return;
  const owner = await readSecure(BACKGROUND_OWNER_KEY);
  if (!owner || (await readSecure(choiceKey(owner))) !== "on") {
    await stopBackgroundUpdates();
    return;
  }
  const session = api().session;
  if (session.state.status !== "signedIn") await session.restore();
  const state = session.state;
  if (state.status !== "signedIn" || state.me.user?.id !== owner) {
    // Signed out, or another account is active: not ours to send. (A network failure leaves it be.)
    if (state.status === "signedOut" || state.status === "signedIn") await stopBackgroundUpdates();
    return;
  }
  const newest = positions.reduce((a, b) => (b.recordedAt > a.recordedAt ? b : a));
  // The OS already paces the fixes (distance, deferred batches); only a repeat is dropped.
  if (reportDecision(newest, last, ANY_PACE, Date.now(), true) !== "send") return;
  last = { at: Date.now(), position: newest };
  try {
    await locationClient().reportLocations({
      samples: [locationSample(newest, { ...(await readBattery()), foreground: false })],
    });
  } catch {
    // The next fix supersedes this one.
  }
}

defineBackgroundLocationTask(reportInBackground);
