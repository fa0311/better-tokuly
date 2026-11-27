import type { OwnPosition } from "@tokuly/core/map";
import { AppState } from "react-native";
import { stopBackgroundUpdates } from "#/platform/location";
import { writeSecure } from "#/platform/secure-storage";
import { BACKGROUND_OWNER_KEY, choiceKey } from "../sharing";

// The device, at the edge: the OS hands its fixes to the task the app defined.
let deliver: (positions: OwnPosition[]) => Promise<void> = async () => undefined;
jest.mock("#/platform/location", () => ({
  defineBackgroundLocationTask: jest.fn((onFixes: typeof deliver) => {
    deliver = onFixes;
  }),
  stopBackgroundUpdates: jest.fn(async () => undefined),
  readBattery: jest.fn(async () => ({})),
}));

/** The server, at the edge: every call is over the limit, for longer than a background task lives. */
const calls: string[] = [];
jest.mock("expo/fetch", () => ({
  fetch: async (input: string) => {
    calls.push(new URL(input).pathname.split("/").pop() ?? "");
    return new Response(JSON.stringify({ code: "resource_exhausted", message: "busy" }), {
      status: 429,
      headers: {
        "content-type": "application/json",
        "x-tokuly-error": "RATE_LIMITED",
        "retry-after": "60",
      },
    });
  },
}));

const fix: OwnPosition = {
  lat: 35.68,
  lng: 139.76,
  accuracyMeters: 10,
  speedMps: 0,
  headingDegrees: 0,
  recordedAt: Date.now(),
};

describe("a fix in the background while the API is rate-limited", () => {
  it("is dropped at once, and sharing stays on", async () => {
    await writeSecure("tokuly.session.active", "rt-alice");
    await writeSecure(BACKGROUND_OWNER_KEY, "alice");
    await writeSecure(choiceKey("alice"), "on");
    AppState.currentState = "background";
    require("../background-report");

    await deliver([fix]);
    await deliver([fix]);

    // Nothing waited for, nobody signed out: the next fix tries again.
    expect(calls).toEqual(["RefreshToken", "RefreshToken"]);
    expect(stopBackgroundUpdates).not.toHaveBeenCalled();
  });
});
