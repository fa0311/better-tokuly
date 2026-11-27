// Device modules the screens touch, mocked at the edge.
jest.mock("expo-secure-store", () => {
  const store = new Map<string, string>();
  return {
    AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY: 0,
    getItemAsync: jest.fn(async (key: string) => store.get(key) ?? null),
    setItemAsync: jest.fn(async (key: string, value: string) => void store.set(key, value)),
    deleteItemAsync: jest.fn(async (key: string) => void store.delete(key)),
  };
});

// A Japanese phone: the app's first language.
jest.mock("expo-localization", () => ({
  getLocales: () => [{ languageTag: "ja-JP" }],
  getCalendars: () => [{ timeZone: "Asia/Tokyo" }],
}));

// LiveKit's native side (WebRTC, the system's audio session) of rooms and hangouts.
jest.mock("@livekit/react-native", () => ({
  registerGlobals: jest.fn(),
  AndroidAudioTypePresets: { communication: {}, media: {} },
  AudioSession: {
    configureAudio: jest.fn(async () => {}),
    startAudioSession: jest.fn(async () => {}),
    stopAudioSession: jest.fn(async () => {}),
  },
}));

// The phone's network state: online, and nothing changes unless a test says so.
jest.mock("expo-network", () => ({
  addNetworkStateListener: jest.fn(() => ({ remove: jest.fn() })),
}));
