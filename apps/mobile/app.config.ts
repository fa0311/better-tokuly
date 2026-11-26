import type { ConfigContext, ExpoConfig } from "expo/config";

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
      UIBackgroundModes: ["location"],
    },
  },
  android: {
    package: "app.tokuly.mobile",
    permissions: [
      "ACCESS_COARSE_LOCATION",
      "ACCESS_FINE_LOCATION",
      "CAMERA",
      "RECORD_AUDIO",
      "POST_NOTIFICATIONS",
    ],
    ...(googleServicesFile ? { googleServicesFile } : {}),
  },
  plugins: [
    "expo-router",
    "expo-secure-store",
    "expo-localization",
    "expo-web-browser",
    "@maplibre/maplibre-react-native",
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
          "記憶やショートの動画に音を入れるために使います。/ Records sound with the videos of your stories and shorts.",
      },
    ],
    [
      "expo-location",
      {
        locationWhenInUsePermission:
          "ともだちのマップにあなたのいまを出すために使います。/ Shows where you are on your friends' map.",
        locationAlwaysAndWhenInUsePermission:
          "アプリを閉じている間も、ともだちにいまを届けるために使います。/ Keeps your friends up to date while the app is closed.",
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
