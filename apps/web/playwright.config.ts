import { defineConfig } from "@playwright/test";

/**
 * Browser tests against a running stack: the web app at `TOKULY_WEB_URL`
 * (default http://localhost:3000) with the API behind `/api`. They create
 * their own accounts and never depend on existing data, so they can run
 * against any environment where sign-up is open.
 *
 *   pnpm --filter @tokuly/web e2e                       # everything, phone and desktop
 *   pnpm --filter @tokuly/web e2e stamps --project=phone
 */
// Playwright empties its output directory when a run starts. Several runs on
// one machine (people or agents working side by side) must not wipe each
// other's traces, so outside CI each run gets a directory of its own. The
// name is fixed here once: worker processes inherit it from the environment.
process.env.TOKULY_E2E_RUN ??= `run-${Date.now()}`;

export default defineConfig({
  testDir: "./e2e",
  outputDir: process.env.CI ? "test-results" : `test-results/${process.env.TOKULY_E2E_RUN}`,
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  timeout: 45_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: process.env.TOKULY_WEB_URL ?? "http://localhost:3000",
    channel: "chrome",
    locale: "ja-JP",
    // The device's time zone. Accounts are on Asia/Tokyo unless a test changes that; running with
    // `TOKULY_E2E_TIMEZONE=Pacific/Kiritimati` (or `Pacific/Pago_Pago`) puts the device on another
    // day than the account, which nothing the app shows may depend on.
    timezoneId: process.env.TOKULY_E2E_TIMEZONE ?? "Asia/Tokyo",
    // Shibuya crossing, so map and distance features have somewhere to be.
    geolocation: { latitude: 35.6595, longitude: 139.7005, accuracy: 12 },
    permissions: ["geolocation", "notifications"],
    trace: "retain-on-failure",
    launchOptions: {
      // Software WebGL so the map renders on machines without a GPU;
      // containers often have a tiny /dev/shm, which crashes renderers.
      args: [
        "--use-angle=swiftshader",
        "--enable-unsafe-swiftshader",
        "--ignore-gpu-blocklist",
        "--enable-webgl",
        "--disable-dev-shm-usage",
      ],
    },
  },
  projects: [
    { name: "phone", use: { viewport: { width: 390, height: 844 }, hasTouch: true } },
    { name: "desktop", use: { viewport: { width: 1280, height: 800 } } },
  ],
});
