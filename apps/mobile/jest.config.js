/** Component tests (jest-expo + React Native Testing Library); logic is tested in packages/. */
module.exports = {
  preset: "jest-expo",
  setupFilesAfterEnv: ["<rootDir>/jest.setup.ts"],
  moduleNameMapper: {
    "^#/(.*)$": "<rootDir>/src/$1",
    "^(\\.{1,2}/.*)\\.js$": "$1",
    // The icons' React Native build is ES modules (.mjs), which the preset does not transform.
    "^lucide-react-native$": require.resolve("lucide-react-native"),
  },
  // pnpm keeps packages under node_modules/.pnpm; the native ones ship untranspiled sources.
  transformIgnorePatterns: [
    "node_modules/(?!(?:\\.pnpm/[^/]+/node_modules/)?(?:(?:jest-)?react-native|@react-native(?:-community)?|expo(?:nent)?|@expo(?:nent)?/.*|@expo-google-fonts/.*|react-navigation|@react-navigation/.*|@maplibre/.*|@shopify/.*|nativewind|react-native-css-interop|lucide-react-native|@bufbuild/.*|@connectrpc/.*|@tokuly/.*))",
  ],
  testPathIgnorePatterns: ["/node_modules/", "/maestro/"],
};
