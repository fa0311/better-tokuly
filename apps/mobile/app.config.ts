import type { ConfigContext, ExpoConfig } from "expo/config";

/**
 * The open-source notices the app shows (Settings → licences), written from
 * what this build bundles every time the config is read: by every build,
 * export and dev server (`scripts/licenses.js`).
 */
const { writeLicenses } = require("./scripts/licenses.js") as { writeLicenses(): number };
writeLicenses();

/**
 * The Tokuly server this build talks to: one origin that serves `/api`,
 * `/rt/ws`, `/media`, `/tiles` and `/map-assets` (the edge in production,
 * the web dev server's proxy in development). Read at build time.
 */
const serverUrl = process.env.EXPO_PUBLIC_TOKULY_URL ?? "http://localhost:3000";

/**
 * Push goes through the phones' own services, not Expo's: FCM on Android
 * (the Firebase project's `google-services.json`, given by path in
 * `GOOGLE_SERVICES_JSON`; without it the build has no push and the app says
 * so) and APNs on iOS (`EXPO_PUBLIC_APNS_ENV`: `production` for store builds).
 */
const googleServicesFile = process.env.GOOGLE_SERVICES_JSON;
const apnsMode = process.env.EXPO_PUBLIC_APNS_ENV === "production" ? "production" : "development";

/**
 * The native projects are generated (`expo prebuild`, Continuous Native
 * Generation) from this file: everything native is a config plugin here.
 */
export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: "Tokuly",
  slug: "tokuly",
  scheme: "tokuly",
  version: "0.1.0",
  orientation: "portrait",
  userInterfaceStyle: "automatic",
  backgroundColor: "#f4f1ff",
  ios: {
    bundleIdentifier: "app.tokuly.mobile",
    supportsTablet: true,
    infoPlist: {
      // Rooms and hangouts keep sounding with the screen off.
      UIBackgroundModes: ["location", "audio"],
    },
  },
  android: {
    package: "app.tokuly.mobile",
    permissions: [
      "ACCESS_COARSE_LOCATION",
      "ACCESS_FINE_LOCATION",
      "CAMERA",
      "RECORD_AUDIO",
      // Rooms and hangouts (WebRTC): the route of the sound, Bluetooth headsets.
      "MODIFY_AUDIO_SETTINGS",
      "BLUETOOTH_CONNECT",
      "POST_NOTIFICATIONS",
    ],
    ...(googleServicesFile ? { googleServicesFile } : {}),
  },
  plugins: [
    "expo-router",
    // Secure storage holds the sessions' tokens without Face ID: no prompt for it.
    ["expo-secure-store", { faceIDPermission: false }],
    "expo-localization",
    "expo-web-browser",
    "@maplibre/maplibre-react-native",
    // Long videos keep playing in a small window (and on the lock screen) outside the app.
    ["expo-video", { supportsPictureInPicture: true, supportsBackgroundPlayback: true }],
    // Rooms and hangouts: LiveKit's WebRTC set up for voice chat.
    ["@livekit/react-native-expo-plugin", { android: { audioType: "communication" } }],
    ["expo-notifications", { mode: apnsMode, color: "#7c5cff" }],
    // The camera of stories and shorts is VisionCamera; these texts are the system's
    // prompts for it and for the library (the image picker's plugin writes them).
    [
      "expo-image-picker",
      {
        photosPermission:
          "トークで送る写真や、記憶・ショートにする写真と動画をえらぶために使います。/ Lets you pick photos and videos for chats, stories and shorts.",
        cameraPermission:
          "トークの写真や、記憶・ショートを撮るために使います。/ Lets you take photos and videos for chats, stories and shorts.",
        microphonePermission:
          "ルームやたまり場で話すときと、記憶やショートの動画に音を入れるときに使うよ！/ Lets you talk in rooms and hangouts, and records sound with the videos of your stories and shorts.",
      },
    ],
    [
      "expo-location",
      {
        locationWhenInUsePermission:
          "ともだちのマップにあなたのいまを出すために使います。/ Shows where you are on your friends' map.",
        locationAlwaysAndWhenInUsePermission:
          "アプリを閉じている間も、ともだちにいまを届けるために使います。/ Keeps your friends up to date while the app is closed.",
        locationAlwaysPermission:
          "アプリを閉じている間も、ともだちにいまを届けるために使います。/ Keeps your friends up to date while the app is closed.",
        locationFullAccuracyPermission:
          "ともだちのマップにあなたのいまをぴったり出すために使います!/ Shows your friends exactly where you are.",
        // The background task pauses while the phone stays still.
        motionUsagePermission:
          "止まっている間は位置の送信をお休みして、電池を長持ちさせるために使います。/ Pauses location updates while you stay still, to save battery.",
        isIosBackgroundLocationEnabled: true,
        isAndroidBackgroundLocationEnabled: true,
        isAndroidForegroundServiceEnabled: true,
      },
    ],
    [
      "expo-build-properties",
      // A development or test stack is plain HTTP; a release against production is not.
      { android: { usesCleartextTraffic: serverUrl.startsWith("http://") } },
    ],
  ],
  experiments: { reactCompiler: true },
});
